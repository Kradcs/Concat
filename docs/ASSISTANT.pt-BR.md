# Assistente de edição integrado

Abra um projeto e clique em **Assistente** na barra superior (desktop).
O assistente recebe um pedido em linguagem natural, propõe comandos do motor
e os aplica à timeline aberta após **Aplicar proposta**. Ctrl+Z desfaz toda a
proposta como uma única edição. Fotos importadas podem ser organizadas,
cortadas na timeline e transformadas como os vídeos.

## Usar sua assinatura ChatGPT Plus

O instalador Windows x64 inclui o Codex nativo da OpenAI. Não é preciso instalar
Node.js nem criar uma chave de API.

1. Abra um projeto e clique em **Assistente**.
2. Na opção **Conta ChatGPT**, clique em **Entrar com ChatGPT**.
3. Conclua o login no navegador usando a conta em que você assina o Plus.
4. Quando aparecer **Conectado ao ChatGPT via Codex**, descreva sua edição.
5. Revise a proposta e clique em **Aplicar proposta**. Ctrl+Z desfaz.

Os pedidos usam a cota de Codex incluída na assinatura. Não são ilimitados:
ao atingir os limites, aguarde a renovação da cota. Este modo não usa sua
chave de API nem muda automaticamente para uma API paga. O modelo é escolhido
pelo Codex de acordo com a disponibilidade da conta.

O login é conduzido pelo Codex oficial, que armazena e renova a sessão no
computador. O Concat não lê os tokens nem sua senha. O armazenamento de
credenciais usa o cofre do sistema quando disponível, com fallback para um
arquivo privado. A pasta separada é `%LOCALAPPDATA%\Concat\codex`; o login de
outra instalação do Codex não é alterado. Use **Sair da conta** para desconectar
o Concat. É possível cancelar o login ou a geração da proposta pelo aplicativo.

Cada pedido é executado em uma pasta temporária privada, sem histórico de
conversa, com shell, navegação e conectores desativados. O acesso de edição a
arquivos fica bloqueado pelo modo de sandbox somente leitura. A proposta passa
pelas mesmas validações do motor antes de habilitar **Aplicar proposta**.

Referências oficiais: [Codex com seu plano ChatGPT](https://github.com/openai/codex#using-codex-with-your-chatgpt-plan)
e [execução não interativa](https://developers.openai.com/codex/noninteractive).

## API ou modelo local (opcional)

Escolha **API / Ollama** para usar um dos serviços abaixo. A API da OpenAI tem
cobrança separada da assinatura ChatGPT; o modelo local usa seu computador.

- **Ollama local:** instale Ollama, execute `ollama pull qwen2.5:7b` e mantenha
  o serviço em execução com contexto suficiente (recomendado: iniciar com
  `OLLAMA_CONTEXT_LENGTH=32768 ollama serve`). Use `http://localhost:11434/v1/chat/completions`,
  modelo `qwen2.5:7b`, sem chave. Um modelo maior costuma gerar comandos mais
  confiáveis; o computador precisa ter recursos para executá-lo.
- **OpenAI:** use `https://api.openai.com/v1/chat/completions`, modelo `gpt-4.1` (ou outro compatível disponível na sua conta) e sua chave no campo protegido.
- **Outros serviços:** informe a URL completa de chat completions, modelo e
  chave de um serviço compatível com OpenAI. HTTP é aceito só em loopback;
  serviços remotos precisam de HTTPS. Redirecionamentos não são seguidos.

As configurações de API ficam na memória desta sessão. A chave não é salva no projeto
nem em logs. Ao enviar um pedido, o serviço recebe os metadados do projeto,
incluindo textos, nomes e caminhos de mídia. Isso também vale para o modo
ChatGPT. Não recebe os arquivos de vídeo
ou foto; o assistente não faz análise visual dos pixels nesta versão.

## Exemplo

Importe uma foto ou vídeo pelos controles normais do editor e peça:
“Adicione um título de abertura por 3 segundos” ou “Gire o primeiro clipe
90 graus”. Revise a explicação e os comandos. Clique em **Aplicar proposta**.
Se o projeto tiver mudado desde o pedido, gere uma nova proposta.

Pedidos não suportados podem resultar em uma pergunta ou erro de validação.
Importar arquivos, exportar e executar modelos de IA de imagem continuam nos
controles existentes do editor. Não há execução de shell pelo assistente.
Cada pedido usa o projeto atual; não há memória de conversa entre pedidos.

## Testes e builds

De `src/`: `cargo test --locked -p concat-agent` e
`cargo clippy --locked -p concat-agent --all-targets -- -D warnings`.
Os testes usam um serviço HTTP controlado, um subprocesso nativo controlado
e o motor real para verificar login/sair, contexto via stdin, propostas,
cancelamento, timeout, respostas inválidas, transações atômicas e desfazer.
O CI executa os testes no Linux e no Windows e verifica se o Codex oficial
fixado aceita a configuração e exige login. Os instaladores incluem o binário
verificado por SHA-256, que também passa por inicialização normal, emulação
de Ivy Bridge e verificação de instruções. Esses testes não fazem login em
uma conta real e não medem a qualidade das respostas de um modelo real.

O workflow **Assistant preview download** compila os instaladores Windows x64,
usa as verificações de inicialização do workflow existente e publica uma
pré-release `assistant-<commit>` somente se todos os builds passarem.
A execução requer GitHub Actions habilitado e permissão para criar releases.
