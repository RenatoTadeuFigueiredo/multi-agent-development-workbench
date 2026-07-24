# Operação do Provider Claude Code

Este runbook cobre a Feature 005: o provider oficial do Claude Code
supervisionado e somente leitura. Ele não cobre rotas pela API da
Anthropic/OpenRouter, login do Claude, workflows com escrita ou MCP
compartilhado.

## Responsabilidade e configuração

Instale e autentique a CLI oficial do Claude Code fora do Workbench. Configure
o executável real e versionado, não um symlink nem um nome resolvido pelo
`PATH`:

```yaml
providers:
  claude:
    type: subscription-cli
    driver: claude-code
    executable: /caminho/absoluto/para/claude-versionado

models:
  specification:
    provider: claude
    runtime_model: fable

roles:
  product-architect:
    model: specification
```

O arquivo precisa ser um executável regular pertencente ao usuário atual. Ele
e os componentes do caminho não podem permitir escrita pelo grupo ou por
outros usuários. O Workbench nunca inicia login, lê o armazenamento de
credenciais do provider nem recebe um token OAuth.

## Lock e inicialização

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- daemon
```

Em outro terminal, verifique a saúde do adaptador e inicie uma sessão roteada:

```bash
cargo run -p workbench-cli -- --json status
cargo run -p workbench-cli -- --json session create
cargo run -p workbench-cli -- prompt <session-id> \
  --role product-architect "Inspecione o repositório e resuma sua arquitetura."
cargo run -p workbench-cli -- status <session-id>
cargo run -p workbench-cli -- session attach <session-id> --after 0
```

O status global deve mostrar o adaptador `claude` como `available` antes do
dispatch. Um estado indisponível expõe apenas uma categoria redigida; dados do
processo do provider nunca devem ser adicionados aos logs.

A geração do lock cria um snapshot privado, executa probes limitados de
`--version` e `auth status --json` e fixa
`claude-code-stream-json/1`, versão e SHA-256. A autenticação precisa indicar
um login de assinatura Claude first-party já existente. Rotas por chave de API
ou cloud alternativo são recusadas para este provider.

Cada prompt recebe um processo novo no workspace canônico. O perfil fixo usa
stream JSON bidirecional, mensagens parciais, modo seguro, `dontAsk`, nenhuma
persistência de transcrição, Chrome e slash commands desativados, manifesto MCP
estritamente vazio e somente `Read`, `Glob` e `Grep`.
`DISABLE_AUTOUPDATER=1` é fixo. Chaves de API, tokens, endpoints alternativos e
seletores Bedrock/Vertex/Foundry herdados são removidos.

## Limites de cobrança e uso

A Anthropic controla elegibilidade de autenticação, uso permitido e cobrança.
Na revisão da Feature 005 em 24/07/2026, `claude -p`, Agent SDK e aplicações
terceiras consomem os limites da assinatura. A Anthropic pausou em 15/06/2026
o crédito separado anunciado para o Agent SDK. Essas regras podem mudar. O
Workbench não garante que um plano cubra uma operação nem intermedeia a
assinatura de um usuário para outro.

Para produtos distribuídos, uso por API ou um modo de cobrança não elegível
para este adaptador local, utilize um provider separado pela Anthropic Console
ou OpenRouter quando esse adaptador estiver implementado. Consulte a
[orientação legal do Claude Code](https://code.claude.com/docs/en/legal-and-compliance)
e a [documentação da CLI](https://code.claude.com/docs/en/cli-usage) atuais
antes do uso em produção. Consulte também a
[orientação atual do plano](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan).

## Atualização e rollback

O Workbench nunca executa comandos de instalação ou atualização do Claude
Code. Para aceitar uma atualização:

1. Conclua ou reconcilie tentativas ativas e pare o daemon do workspace.
2. Preserve o executável anterior ou o método de rollback do provider.
3. Atualize pelo processo oficial fora do Workbench.
4. Resolva o novo executável real e revise mudanças de versão, procedência,
   permissões, ferramentas, MCP, plugins, skills, browser, persistência,
   protocolo e cobrança.
5. Regenere o lock, valide a configuração, execute `make check` e confira a
   saúde do adaptador antes do primeiro prompt.

Restaure o executável anterior e o lock correspondente se a compatibilidade
falhar. Nunca edite manualmente um digest ou a identidade do protocolo.

## Falhas e cancelamento

| Condição | Ação necessária |
|---|---|
| Autenticação da assinatura indisponível | Autentique na CLI oficial fora do Workbench, reinicie e verifique novamente. Não adicione credenciais à configuração. |
| Divergência de digest ou versão | Confirme que a atualização foi intencional e gere outro lock após revisão, ou restaure o executável fixado. |
| Falha de inicialização ou capacidade | Restaure a CLI compatível anterior. Não ignore o lock nem o perfil seguro. |
| Crash, EOF, frame inválido ou cancelamento incompleto após o dispatch | Trate como `outcome_unknown`; examine o histórico durável e reconcilie manualmente. Não repita automaticamente. |

O cancelamento somente é confirmado após a resposta correlacionada do
interrupt e um resultado com `aborted_streaming` ou `aborted_tools`. Apenas
acknowledgement, erro, silêncio, EOF ou término do processo não bastam.

## Validação sem quota

O conjunto padrão executa somente o fake versionado no repositório:

```bash
make test-claude
make check
```

O smoke live ignorado por padrão executa apenas autenticação, inicialização e
confirmação de recebimento do interrupt; nenhuma mensagem de usuário é enviada
e nenhum turno do modelo é iniciado:

```bash
CARGO_NET_OFFLINE=true \
WORKBENCH_CLAUDE_EXECUTABLE=/caminho/absoluto/para/claude-versionado \
WORKBENCH_CLAUDE_VERSION=2.1.218 \
cargo test -p workbench-claude --test live_claude --locked -- \
  --ignored --exact exact_profile_initializes_without_sending_a_user_message
```

Substitua `2.1.218` pela versão normalizada exata selecionada pelo lock. O
smoke verifica somente compatibilidade; ele não comprova lock, digest, snapshot
ou procedência de produção. Inferência exige autorização separada do operador.
Nunca anexe a issues ou registros de compatibilidade stdout, stderr, campos de
autenticação, thinking, usage, dados de ferramentas, identificadores do
provider ou valores de ambiente.
