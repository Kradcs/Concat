// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! The embedded assistant edits the window's session, never a second API session.
use crate::host::{self, Shell};
use crate::studio::Studio;
use crate::ui::App;
use concat_agent::{Config, Plan};
use concat_project::doc::to_document;
use serde_json::Value;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct Pending {
    epoch: u64,
    snapshot: Value,
    plan: Plan,
}
thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static CANCEL: RefCell<Option<Arc<Operation>>> = const { RefCell::new(None) };
}

struct Operation {
    cancel: AtomicBool,
    finished: AtomicBool,
}

fn begin() -> Arc<Operation> {
    let cancel = Arc::new(Operation {
        cancel: AtomicBool::new(false),
        finished: AtomicBool::new(false),
    });
    CANCEL.with(|slot| *slot.borrow_mut() = Some(cancel.clone()));
    cancel
}

pub fn stop() {
    let active = CANCEL.with(|slot| slot.borrow().clone());
    if let Some(active) = active {
        active.cancel.store(true, Ordering::Relaxed);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !active.finished.load(Ordering::Relaxed) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

fn account(app: &App, action: u8) {
    if app.get_assistant_busy() {
        return;
    }
    app.set_assistant_busy(true);
    app.set_assistant_account_status(match action {
        1 => "Conclua o login no navegador que será aberto. Você pode cancelar aqui.".into(),
        2 => "Saindo da conta…".into(),
        _ => "Verificando sua conta…".into(),
    });
    let cancel = begin();
    host::spawn(
        move || {
            let result = (|| {
                let codex = concat_agent::codex::Codex::bundled()?;
                match action {
                    1 => codex.login(&cancel.cancel)?,
                    2 => codex.logout(&cancel.cancel)?,
                    _ => {}
                }
                codex.signed_in(&cancel.cancel)
            })();
            cancel.finished.store(true, Ordering::Relaxed);
            result
        },
        move |_, app, _, result: Result<bool, String>| {
            CANCEL.with(|slot| *slot.borrow_mut() = None);
            app.set_assistant_busy(false);
            match result {
                Ok(signed_in) => {
                    app.set_assistant_signed_in(signed_in);
                    app.set_assistant_account_status(if signed_in {
                        "Conectado ao ChatGPT via Codex. Os pedidos usam os limites do seu plano."
                            .into()
                    } else {
                        "Entre com sua conta do ChatGPT. O login será feito no navegador da OpenAI."
                            .into()
                    });
                }
                Err(error) => {
                    app.set_assistant_signed_in(false);
                    app.set_assistant_account_status(error.into());
                }
            }
        },
    );
}
fn snapshot(studio: &Studio) -> Option<Value> {
    studio
        .session
        .as_ref()
        .map(|session| to_document(&session.settings(), session.project()))
}
pub fn bind(app: &App) {
    app.on_assistant_refresh(|| Shell::with(|_, app| account(&app, 0)));
    app.on_assistant_login(|| Shell::with(|_, app| account(&app, 1)));
    app.on_assistant_logout(|| Shell::with(|_, app| account(&app, 2)));
    app.on_assistant_cancel(|| {
        CANCEL.with(|slot| {
            if let Some(cancel) = slot.borrow().as_ref() {
                cancel.cancel.store(true, Ordering::Relaxed);
            }
        });
    });
    app.on_assistant_ask(|| {
        Shell::with(|shell, app| {
            if app.get_assistant_busy() {
                return;
            }
            PENDING.with(|slot| *slot.borrow_mut() = None);
            app.set_assistant_can_apply(false);
            let Some(document) = snapshot(&shell.studio.borrow()) else {
                app.set_assistant_reply("Abra um projeto antes de pedir uma edição.".into());
                return;
            };
            let context = {
                let studio = shell.studio.borrow();
                serde_json::json!({
                    "document": &document,
                    "selectedClipIds": &studio.selection,
                    "playheadSeconds": studio.playhead,
                })
            };
            let epoch = host::project_epoch();
            let config = Config {
                endpoint: app.get_assistant_endpoint().to_string(),
                model: app.get_assistant_model().to_string(),
                key: app.get_assistant_key().to_string(),
            };
            let prompt = app.get_assistant_prompt().to_string();
            let chatgpt = app.get_assistant_chatgpt();
            let cancel = begin();
            app.set_assistant_busy(true);
            app.set_assistant_reply("Preparando proposta…".into());
            host::spawn(
                move || {
                    let result = if chatgpt {
                        concat_agent::codex::Codex::bundled()
                            .and_then(|codex| codex.propose(&prompt, &context, &cancel.cancel))
                    } else {
                        concat_agent::propose(config, &prompt, &context)
                    };
                    cancel.finished.store(true, Ordering::Relaxed);
                    (document, result)
                },
                move |studio, app, _, (document, result)| {
                    CANCEL.with(|slot| *slot.borrow_mut() = None);
                    app.set_assistant_busy(false);
                    if epoch != host::project_epoch()
                        || snapshot(studio).as_ref() != Some(&document)
                    {
                        app.set_assistant_reply(
                            "O projeto mudou. Envie o pedido novamente.".into(),
                        );
                        return;
                    }
                    match result {
                        Ok(plan) => {
                            let check = concat_project::editor::Editor::from_document(&document)
                                .ok_or_else(|| "Não foi possível validar o projeto.".to_string())
                                .and_then(|mut editor| {
                                    editor
                                        .apply(concat_project::Command::Batch {
                                            commands: plan.commands.clone(),
                                        })
                                        .map_err(|e| e.to_string())
                                        .and_then(|outcome| {
                                            if outcome.applied || plan.commands.is_empty() {
                                                Ok(())
                                            } else {
                                                Err("A proposta não altera o projeto. Peça uma edição mais específica.".into())
                                            }
                                        })
                                });
                            if let Err(error) = check {
                                app.set_assistant_reply(
                                    format!("Proposta inválida: {error}").into(),
                                );
                                return;
                            }
                            app.set_assistant_reply(plan.preview().into());
                            app.set_assistant_can_apply(!plan.commands.is_empty());
                            PENDING.with(|slot| {
                                *slot.borrow_mut() = Some(Pending {
                                    epoch,
                                    snapshot: document,
                                    plan,
                                })
                            });
                        }
                        Err(error) => app.set_assistant_reply(error.into()),
                    }
                },
            );
        })
    });
    app.on_assistant_apply(|| {
        Shell::with(|shell, app| {
            let pending = PENDING.with(|slot| slot.borrow_mut().take());
            app.set_assistant_can_apply(false);
            let Some(pending) = pending else {
                return;
            };
            let mut studio = shell.studio.borrow_mut();
            if pending.epoch != host::project_epoch()
                || snapshot(&studio).as_ref() != Some(&pending.snapshot)
            {
                app.set_assistant_reply(
                    "O projeto mudou. Envie o pedido novamente antes de aplicar.".into(),
                );
                return;
            }
            studio.apply(pending.plan.into_command());
            studio.refresh_art();
            studio.publish(&app, &shell.models);
            app.set_assistant_reply(
                "Proposta aplicada à timeline. Use Desfazer para reverter.".into(),
            );
        })
    });
}
