// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
use super::*;
use concat_project::editor::Editor;
use std::io::{Read, Write};
use std::net::TcpListener;

fn title() -> Value {
    json!({"op":"addTextClip","trackId":null,"start":0.0,"style":null,"duration":3.0})
}
fn plan(commands: Value) -> String {
    json!({"message":"Proposta de edição","commands":commands}).to_string()
}

#[test]
fn proposed_edits_apply_and_undo_as_one_transaction() {
    let mut editor = Editor::new();
    let before = editor.project().clone();
    let proposed = parse_plan(&plan(json!([title(), title()]))).unwrap();
    assert_eq!(editor.project(), &before, "proposing must not edit");
    editor.apply(proposed.into_command()).unwrap();
    assert_eq!(editor.project().active().clips.len(), 2);
    assert!(editor.undo());
    assert_eq!(editor.project(), &before);
}

#[test]
fn a_failed_edit_does_not_leave_partial_changes() {
    let mut editor = Editor::new();
    let before = editor.project().clone();
    let proposed = parse_plan(&plan(
        json!([title(), {"op":"mergeClips","clipIds":["missing"]}]),
    ))
    .unwrap();
    assert!(editor.apply(proposed.into_command()).is_err());
    assert_eq!(editor.project(), &before);
    assert!(!editor.can_undo());
}

#[test]
fn rejects_malformed_or_excessive_plans_and_nested_batches() {
    for content in [
        "not JSON".to_string(),
        plan(json!([{"op":"invented"}])),
        plan(json!([{"op":"batch","commands":[]}])),
        plan(json!([{"op":"removeMedia","mediaId":"m1"}])),
        plan(Value::Array(vec![title(); 33])),
    ] {
        assert!(parse_plan(&content).is_err());
    }
    let fenced = format!("```json\n{}\n```", plan(json!([])));
    assert!(parse_plan(&fenced).unwrap().commands.is_empty());
}

#[test]
fn refuses_insecure_remote_endpoints_before_sending_credentials() {
    let config = Config {
        endpoint: "http://example.com/v1/chat/completions".into(),
        model: "test".into(),
        key: "private-key".into(),
    };
    assert!(validate(&config, "edit").is_err());
    let config = Config {
        endpoint: "https://user:secret@example.com/v1/chat/completions".into(),
        ..config
    };
    assert!(validate(&config, "edit").is_err());
}

#[test]
fn http_round_trip_contains_the_project_and_applies_real_commands() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut byte = [0u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
        }
        let headers = String::from_utf8(bytes).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["model"], "test-model");
        assert!(
            request["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("Adicione um título")
        );
        assert!(
            request["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("addTextClip")
        );
        let response =
            json!({"choices":[{"message":{"content":plan(json!([title()]))}}]}).to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
    });
    let proposed = propose(
        Config {
            endpoint: format!("http://{address}/v1/chat/completions"),
            model: "test-model".into(),
            key: String::new(),
        },
        "Adicione um título",
        &json!({"clips":[]}),
    )
    .unwrap();
    server.join().unwrap();
    let mut editor = Editor::new();
    editor.apply(proposed.into_command()).unwrap();
    assert_eq!(editor.project().active().clips.len(), 1);
    assert_eq!(editor.project().active().clips[0].duration, 3.0);
}

#[test]
fn schema_describes_only_available_editing_operations() {
    let schema = editing_schema();
    let alternatives = schema["oneOf"].as_array().unwrap();
    assert!(alternatives.len() >= 10);
    for alternative in alternatives {
        assert!(supported(
            alternative["properties"]["op"]["const"].as_str().unwrap()
        ));
    }
    let serialized = schema.to_string();
    assert!(!serialized.contains("\"const\":\"addMedia\""));
    assert!(
        !serialized.contains("#/$defs/Command"),
        "nested batches are unavailable"
    );
}

#[test]
fn edits_both_imported_video_and_photo_clips() {
    let mut editor = Editor::new();
    let mut ids = Vec::new();
    for (kind, name) in [("video", "video.mp4"), ("image", "photo.png")] {
        let media: Command = serde_json::from_value(json!({"op":"addMedia","item":{
            "path":format!("/media/{name}"),"name":name,"kind":kind,"duration":10.0,
            "width":320,"height":240,"hasAudio":false
        }}))
        .unwrap();
        let media_id = editor.apply(media).unwrap().created_id.unwrap();
        let command: Command = serde_json::from_value(
            json!({"op":"addClipAtFirstFree","mediaId":media_id,"start":0.0}),
        )
        .unwrap();
        ids.push(editor.apply(command).unwrap().created_id.unwrap());
    }
    let before = editor.project().clone();
    let commands: Vec<Value> = ids
        .iter()
        .map(|id| json!({"op":"setClipTransform","clipId":id,"rotation":90.0,"scale":1.5}))
        .collect();
    editor
        .apply(parse_plan(&plan(json!(commands))).unwrap().into_command())
        .unwrap();
    for clip in &editor.project().active().clips {
        assert_eq!(clip.rotation, 90.0);
        assert_eq!(clip.scale, 1.5);
    }
    assert!(editor.undo());
    assert_eq!(editor.project(), &before);
}
