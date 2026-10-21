# Assistente de edição integrado

Abra um projeto e clique em **Assistente** na barra superior (desktop).
O assistente recebe um pedido em linguagem natural, propõe comandos do motor
e os aplica à timeline aberta após **Aplicar proposta**. Ctrl+Z desfaz toda a
proposta como uma única edição. Fotos importadas podem ser organizadas,
cortadas na timeline e transformadas como os vídeos.

## Escolher um modelo

- **Ollama local:** instale Ollama, execute `ollama pull qwen2.5:7b` e mantenha
  o serviço em execução com contexto suficiente (recomendado: iniciar com
  `OLLAMA_CONTEXT_LENGTH=32768 ollama serve`). Use `http://localhost:11434/v1/chat/completions`,
  modelo `qwen2.5:7b`, sem chave. Um modelo maior costuma gerar comandos mais
  confiáveis; o computador precisa ter recursos para executá-lo.
- **OpenAI:** use `https://api.openai.com/v1/chat/completions`, o nome de um
  modelo disponível na sua conta e sua chave no campo protegido.
- **Outros serviços:** informe a URL completa de chat completions, modelo e
  chave de um serviço compatível com OpenAI. HTTP é aceito só em loopback;
  serviços remotos precisam de HTTPS. Redirecionamentos não são seguidos.

As configurações ficam na memória desta sessão. A chave não é salva no projeto
nem em logs. Ao enviar um pedido, o serviço recebe os metadados do projeto,
incluindo textos, nomes e caminhos de mídia. Não recebe os arquivos de vídeo
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
Os testes usam um serviço HTTP controlado e o motor real para verificar a
integração, respostas inválidas, transações atômicas e desfazer. Esses testes
não medem a qualidade das respostas de um modelo real.

O workflow **Assistant preview download** compila os instaladores de desktop,
usa as verificações de inicialização do workflow existente e publica uma
pré-release `assistant-<commit>` somente se todos os builds passarem.
A execução requer GitHub Actions habilitado e permissão para criar releases.
