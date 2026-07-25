# Workbench de Desenvolvimento Multiagente

![Claude, Codex, Grok e OpenRouter conectados a um núcleo portátil de orquestração em Rust](assets/readme-hero.svg)

<p align="center">
  <img alt="Status do projeto: MVP do monorepo completo (Features 001–016)" src="https://img.shields.io/badge/status-monorepo%20MVP%20complete-16A34A">
  <img alt="Plano de controle: Features 001–016 entregues" src="https://img.shields.io/badge/control%20plane-001%E2%80%93016-2563EB">
  <img alt="Linguagem do núcleo: Rust" src="https://img.shields.io/badge/core-Rust-DEA584?logo=rust&logoColor=111827">
  <img alt="Interface principal: VS Code" src="https://img.shields.io/badge/interface-VS%20Code-007ACC?logo=visualstudiocode&logoColor=white">
  <img alt="Licença: Apache 2.0" src="https://img.shields.io/github/license/RenatoTadeuFigueiredo/multi-agent-development-workbench?color=2563EB">
  <img alt="Último commit" src="https://img.shields.io/github/last-commit/RenatoTadeuFigueiredo/multi-agent-development-workbench?color=475569">
</p>

<p align="center">
  <a href="README.md">English</a>
  ·
  <a href="README.pt-BR.md"><strong>Português Brasileiro</strong></a>
  ·
  <a href="CONTRIBUTING.md">Contribuição</a>
  ·
  <a href="SECURITY.md">Segurança</a>
  ·
  <a href="LICENSE">Licença</a>
</p>

<p align="center">
  <strong>Um único workflow. O modelo certo para cada papel de engenharia.</strong>
</p>

> [!IMPORTANT]
> As Features **001–016** entregam o plano de controle do monorepo: núcleo de
> orquestração em Rust, sessões criptografadas e isoladas por workspace, CLI
> headless, ponte fina para o VS Code com controles de workflow em tempo real,
> adaptadores supervisionados Grok/Claude/Codex, gateway MCP central (incluindo
> TLS HTTPS fora de loopback), workflows multiagente configuráveis, OpenRouter
> com controles de custo e ledger de gasto por sessão, `workbench agent stdio`
> (ponte ACP anexada ao daemon em execução), ferramentas nativas de escrita
> sob política central fail-closed e a superfície de launch `WorkbenchBackend`
> para o terminal. O trabalho residual fica fora deste monorepo: o rebase
> dual-upstream do pager Grok Build e a suíte PTY permanecem em
> [grok-build](https://github.com/RenatoTadeuFigueiredo/grok-build).

## Sumário

- [Resumo executivo](#resumo-executivo)
- [Experiência proposta](#experiência-proposta)
- [Arquitetura](#arquitetura)
- [Configuração e roteamento](#configuração-e-roteamento)
- [Prontidão do projeto](#prontidão-do-projeto)
- [Próximos passos](#próximos-passos)
- [Fonte de verdade do Speckit](doc/arch/functional/product-overview.md)

🧭 **Fase atual:** plano de controle do monorepo completo (Features 001–016).
Trabalho ativo no monorepo: manutenção e enablement do operador. Residual de
produto: pager TUI derivado do Grok em `grok-build` (rebase dual-upstream e
pin publicado do fork).

## Resumo executivo

O Workbench de Desenvolvimento Multiagente é um único local para planejar,
executar, revisar e supervisionar trabalhos de desenvolvimento realizados por
diferentes agentes de IA. O plano de controle independente de fornecedores
está **entregue neste monorepo** nas Features 001–016: fakes offline
determinísticos e adaptadores supervisionados para Grok Build (ACP), Claude
Code (stream JSON), Codex (exec JSONL) e OpenRouter (Chat Completions) nos
mesmos contratos orientados a papéis, com MCP central, execução de workflows,
ledger de custos e ACP agent stdio para clientes de editor e terminal.

A interface principal é o **Visual Studio Code**, aproveitando suas sessões
agênticas, APIs de extensão, ferramentas Git e preview nativo de Markdown. A
ponte fina em TypeScript se conecta ao núcleo Rust e pode criar, listar,
selecionar, anexar e acompanhar sessões isoladas por workspace com superfícies
de roteamento, etapas e aprovações em tempo real. `workbench agent stdio`
apresenta sessões do daemon como agente ACP; o monorepo entrega
`WorkbenchBackend` como contrato de launch para um pager derivado do Grok. A
TUI interativa completa permanece como patch no fork
[grok-build](https://github.com/RenatoTadeuFigueiredo/grok-build).
Compatibilidade ACP opcional para outros editores reutiliza os mesmos serviços
do daemon.

O objetivo não é criar outro modelo de IA. É criar um plano de controle independente de fornecedor que faça diferentes agentes existentes trabalharem como uma única equipe de engenharia, com responsabilidades e resultados auditáveis.

## Visão geral

| Um workspace | Roteamento por papéis | Acesso flexível | Portável entre editores |
|---|---|---|---|
| Prompts, progresso, artefatos, diffs e intervenções permanecem juntos. | Claude especifica, Codex revisa, Grok implementa e os workflows continuam configuráveis. | Utilize assinaturas nativas ou modelos por API através do OpenRouter. | Comece no VS Code, continue no terminal e conecte outros editores por ACP. |

**Princípios centrais:** independência de fornecedor · transferências explícitas · controle humano · sessões duráveis · execução auditável · especificação antes da implementação.

## Problema

Trabalhar atualmente com várias ferramentas de IA exige janelas, sessões, prompts, arquivos de instruções e históricos separados. Isso provoca:

- Regras inconsistentes entre fornecedores.
- Perda de contexto durante transferências manuais.
- Prompts duplicados e análise repetida do repositório.
- Edições concorrentes e responsabilidades pouco claras.
- Baixa visibilidade sobre progresso, decisões e falhas.
- Ausência de um ciclo confiável entre especificação, implementação e validação.

## Experiência proposta

O usuário inicia uma única sessão, descreve o resultado desejado e seleciona ou reutiliza um workflow. O orquestrador atribui cada etapa ao agente configurado, transmite o progresso em uma timeline unificada, persiste os artefatos e avança automaticamente.

```mermaid
flowchart LR
    U[Solicitação do usuário] --> C[Claude: especificação]
    C --> R[Codex: revisão e enriquecimento]
    R --> G[Grok: implementação]
    G --> V[Codex: testes e validação]
    V -->|Problemas encontrados| G
    V -->|Aprovado| D[Alteração concluída]
    C -. Revisão humana opcional .-> U
    V -. Dúvida ou ação sensível .-> U
```

O usuário poderá interromper, comentar, pausar, retomar ou redirecionar o workflow a qualquer momento. As aprovações humanas serão configuráveis por workflow; incertezas, requisitos conflitantes e operações sensíveis sempre solicitarão confirmação.

## Interfaces

### Workbench no VS Code

O VS Code será o ambiente principal de trabalho:

- Interfaces nativas de Chat e Agents para prompts, status, sessões e intervenções.
- Agentes personalizados, subagentes e handoffs para trabalho interativo orientado por papéis.
- Preview nativo de Markdown com renderização de Mermaid.
- Código, Git, diffs, testes, depuração e terminais integrados.
- Worktrees Git isoladas e opcionais para tarefas concorrentes.

A ponte implementada do Workbench usa APIs públicas estáveis e o protocolo
local versionado para resolver o endpoint do workspace, criar e selecionar
sessões, anexar, reconectar, controlar e manter em memória um documento
Markdown de eventos. A Feature 009 exibe planos de roteamento, etapas do
workflow e aprovações nesse documento e na barra de status. A lógica dos
workflows e providers permanece em Rust.

As especificações serão armazenadas como arquivos normais do repositório, por exemplo:

```text
doc/arch/
├── sdd/001-nome-da-feature/
│   ├── spec.md
│   ├── plan.md
│   └── tasks.md
├── adr/
├── schemas/
└── specs/features/
```

Os revisores poderão editar os documentos diretamente ou adicionar blocos visíveis de revisão:

```markdown
> [!REVIEW]
> Esclarecer o comportamento quando a rotação falhar depois que o token anterior expirar.
```

### Interface de terminal

O caminho de terminal reutiliza o pager maduro do Grok Build para edição de
prompts, scrollback, Markdown e Mermaid, diffs, aprovações, tarefas, mouse e
comportamento do terminal. A apresentação vive no
[fork do Grok Build do Workbench](https://github.com/RenatoTadeuFigueiredo/grok-build);
este monorepo entrega a superfície de launch ACP e a ponte com o daemon:

```bash
workbench daemon
workbench agent stdio
workbench status
workbench session create
workbench session attach <session-id>
workbench session pause <session-id>
workbench session resume <session-id>
workbench session cancel <session-id>
```

O binário de terminal é um cliente de apresentação, não o orquestrador. Ele
inicia `workbench agent stdio` (Features 011–012), que anexa ao daemon do
workspace e traduz ACP para o protocolo local versionado. `WorkbenchBackend`
(Feature 016) planeja esse launch com caminhos absolutos. O executável oficial
`grok` continua sendo um runtime de provider separado para SuperGrok. A CLI
headless e o JSON estruturado já cobrem scripts e CI; o rebase dual-upstream
do pager interativo permanece em `grok-build`.

## Arquitetura

```mermaid
flowchart TB
    V[Extensão do VS Code] -->|Protocolo local versionado| D[Daemon do Workbench]
    T[TUI do Workbench derivada do Grok] -->|ACP stdio| B[Ponte de terminal do Workbench]
    B -->|Protocolo local versionado| D
    H[CLI headless] --> D
    Z[Cliente ACP do Zed] --> X[Servidor ACP]
    J[Cliente ACP da JetBrains] --> X
    X --> D
    D --> Q[Roteador de intenções]
    Q --> O[Núcleo de orquestração]

    O --> P[Políticas e permissões]
    O --> S[Sessões e eventos]
    O --> A[Gerenciador de artefatos]
    O --> W[Máquina de estados dos workflows]
    O --> M[Gateway MCP central]
    O --> C[Registro de configurações e capacidades]

    W --> CA[Adaptador Claude Code]
    W --> CO[Adaptador Codex CLI/ACP]
    W --> GR[Adaptador Grok Build ACP]
    W --> OA[Agente genérico por API]

    CA --> CL[Assinatura Claude]
    CO --> OP[Assinatura ChatGPT]
    GR --> XA[Assinatura SuperGrok]
    OA --> OR[API do OpenRouter]
```

Toda a lógica de orquestração e da aplicação, além de todos os binários
próprios, é implementada em Rust como um pequeno conjunto de componentes
testáveis de forma independente. O cliente fino do VS Code é a única exceção
de runtime de primeira parte:

- **Mecanismo de workflows:** etapas determinísticas, transições, novas
  tentativas controladas por política e ciclos de revisão.
- **Roteador de intenções:** destinos explícitos, contexto ativo, consultas
  determinísticas e classificação pelo coordenador sem transmissões implícitas.
- **Registro de configurações:** configuração em camadas, aliases estáveis de
  papéis e modelos, verificação de capacidades, lock e snapshots das sessões.
- **Adaptadores de fornecedores:** ciclo de vida dos processos, detecção de autenticação, retomada de sessões, cancelamento e normalização de eventos.
- **Gerenciador de políticas:** instruções compartilhadas, permissões de ferramentas e regras de aprovação.
- **Armazenamento de eventos:** histórico durável e criptografado das sessões e
  trilha de auditoria, inicialmente com SQLite.
- **Gerenciador de artefatos:** especificações, planos, decisões, diferenças e relatórios de validação.
- **Ponte do editor:** protocolo local versionado entre a extensão do VS Code e o daemon Rust.
- **Extensão do VS Code:** adaptador fino de apresentação e comandos, sem lógica de orquestração.
- **Servidor ACP:** compatibilidade com Zed, JetBrains e outros clientes ACP.
- **Ponte ACP de terminal:** apresenta as sessões do daemon como um agente ACP
  com negociação de capacidades para o pager derivado do fork.
- **Cliente de terminal derivado do Grok:** reutiliza o comportamento de
  apresentação upstream sem controlar workflows, credenciais ou políticas.
- **Gateway MCP:** instala, versiona, supervisiona, filtra e audita servidores
  MCP compartilhados por todos os providers compatíveis.
- **CLI headless:** acesso portátil para scripts e CI ao mesmo núcleo.
- **Runtime de agente genérico:** tool calling, gerenciamento de contexto, streaming, limites de custo e aprovações para modelos acessados por API.

## Implementação em Rust

A implementação atual é um workspace Cargo fixado no Rust 1.95:

```text
crates/
├── workbench-core/              # Domínio, roteamento, políticas e ports
├── workbench-config/            # Camadas, validação, snapshots e locks
├── workbench-storage/           # SQLite criptografado, key stores, exportação e gasto
├── workbench-protocol/          # Comandos e eventos NDJSON versionados
├── workbench-daemon/            # Serviços da aplicação e IPC Unix local
├── workbench-cli/               # Ciclo do daemon e comandos headless
├── workbench-acp/               # Cliente ACP v1 de provider e supervisão
├── workbench-acp-server/        # Ponte ACP agent stdio para o protocolo local
├── workbench-claude/            # Stream JSON do Claude e supervisão por tentativa
├── workbench-codex/             # Codex exec JSONL e supervisão por tentativa
├── workbench-openrouter/        # OpenRouter Chat Completions e orçamentos
├── workbench-mcp/               # Gateway MCP central, pins, TLS e allowlists
├── workbench-terminal-backend/  # Contrato de launch WorkbenchBackend para a TUI
└── workbench-testkit/           # Fakes, contratos, aceitação e SLOs
```

Compile e exercite o corte vertical offline:

```bash
make build
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
cargo run -p workbench-cli -- daemon
# em outro terminal:
cargo run -p workbench-cli -- --json status
cargo run -p workbench-cli -- --json session create
```

O
[quickstart E2E do operador](docs/operations/operator-e2e-quickstart.md)
cobre lock, daemon, sessão, workflow, anexo no VS Code, agent stdio e
políticas de custo (offline vs live). Runbooks por provider:
[Grok ACP](docs/operations/grok-acp-provider.md),
[Claude Code](docs/operations/claude-code-provider.pt-BR.md),
[Codex](docs/operations/codex-provider.pt-BR.md) e
[OpenRouter](docs/operations/openrouter-provider.md).

A extensão do VS Code é o único componente próprio fora de Rust, pois extensões
do VS Code executam em um host TypeScript/JavaScript. Ela permanece um cliente
substituível que exibe o estado do daemon e encaminha comandos. A implementação
limitada de ACP JSON-RPC/NDJSON do repositório está isolada em
`workbench-acp`, impedindo que mudanças de protocolo e provider afetem o modelo
de domínio.

O código de apresentação do terminal será mantido separadamente no fork do
Grok Build. Sua branch `main` espelhará exatamente o upstream; a branch
`workbench` carregará a patch stack mínima do backend externo. O repositório
principal do Workbench fixará um commit testado do fork em vez de vendorizar o
pager.

## Configuração e roteamento

Providers, aliases de modelos, papéis, roteamento, políticas e workflows serão
declarativos. Os workflows utilizarão papéis estáveis em vez de modelos
específicos de fornecedores:

```yaml
version: 1

providers:
  claude:
    type: subscription-cli
    driver: claude-code
    executable: /caminho/absoluto/canonico/para/claude-versionado
  grok:
    type: acp
    executable: /caminho/absoluto/canonico/para/grok

models:
  specification:
    provider: claude
    runtime_model: fable
  implementation:
    provider: grok
    runtime_model: grok-4.5
  review-fallback:
    provider: grok
    runtime_model: grok-4.5

roles:
  workspace-coordinator:
    model: review-fallback
  product-architect:
    model: specification
  critical-reviewer:
    model: review-fallback
  implementer:
    model: implementation
  code-reviewer:
    model: review-fallback

routing:
  default_role: workspace-coordinator
  confidence_threshold: 0.85

policies:
  default_tool_mode: read-only
  global_deny: []
  production_mutations: approval-required

workflows:
  feature-delivery:
    steps:
      - id: specification
        role: product-architect
      - id: spec-review
        role: critical-reviewer
      - id: implementation
        role: implementer
      - id: validation
        role: code-reviewer
        on_findings: implementation
        max_iterations: 3
```

Este é um exemplo da camada do repositório. Os padrões seguros preenchem campos
vazios omitidos dos papéis; ferramentas e fontes de dados precisam ser
declaradas antes de serem referenciadas por um papel. A configuração resolvida
é totalmente explícita e deve obedecer ao schema versionado. Os drivers
Claude, Codex, Grok e OpenRouter já embarcam neste monorepo; declare um
provider somente quando o executável ou a credencial estiver instalado e
autenticado nesta estação.

As configurações serão resolvidas a partir dos padrões seguros embutidos, da
configuração do usuário, de `.workbench/workbench.yaml` e dos overrides
explícitos da sessão. Um `.workbench/workbench.lock` determinístico fixará
adaptadores, modelos, MCPs e dados de compatibilidade fora da sessão. Overrides
de sessão criarão um lock vinculado sem reescrever esse arquivo-base. Os
segredos permanecerão no keychain, e os dados sensíveis das sessões serão
criptografados com chaves individuais.

Toda mensagem livre chegará primeiro ao daemon. Destinos explícitos e o
contexto do workflow terão precedência, seguidos por consultas determinísticas
de status/histórico e pelo coordenador configurado. Antes do envio, a interface
mostrará intenção, papel, modelo resolvido, ferramentas, fontes de dados,
permissões e confiança. Mensagens nunca serão transmitidas implicitamente para
vários providers.

Todos os providers implementarão o mesmo contrato de capacidades. Adicionar um
modelo a um adaptador existente ou API compatível exigirá apenas configuração;
um novo protocolo exigirá um adaptador Rust isolado. Remover um provider
validará aliases e fallbacks afetados sem tornar o histórico ilegível. Sessões
ativas manterão um snapshot sem segredos, portanto mudanças de modelo valerão
para novas sessões, salvo migração explícita.

A verificação de capacidades diferenciará modelos de chat de agentes de código
avaliando tool calling, saída estruturada, contexto, privacidade,
disponibilidade, custos, retomada de sessões e compatibilidade de protocolo. A
decisão completa está em
[`docs/architecture/configuration-routing-and-providers.md`](docs/architecture/configuration-routing-and-providers.md).

## Regras e contexto compartilhados

O orquestrador resolverá um conjunto canônico de instruções para cada etapa:

1. Políticas da organização.
2. `AGENTS.md` do repositório.
3. Instruções do repositório compatíveis com cada fornecedor, como `CLAUDE.md`.
4. Instruções do workflow e do papel do agente.
5. Solicitação atual do usuário.

As instruções resultantes ficarão visíveis antes da execução e serão incluídas em cada transferência. Configurações nativas dos fornecedores continuarão sendo suportadas, mas eventuais conflitos serão informados em vez de resolvidos silenciosamente.

## MCPs e ferramentas compartilhadas

O daemon controlará um manifesto e um lockfile MCP canônicos. Instalar ou
atualizar um servidor compartilhado uma única vez fixará a versão do pacote ou
imagem, checksum, transporte, referência de credencial e política. Os agentes
compatíveis se conectarão a um único Gateway MCP do Workbench e receberão
allowlists de ferramentas específicas para cada papel.

Servidores MCP HTTP remotos compartilharão naturalmente um único endpoint.
Servidores stdio locais serão iniciados e supervisionados pelo gateway, evitando
que cada provider baixe ou execute versões diferentes. Credenciais permanecerão
no keychain do sistema ou no ambiente e nunca serão gravadas no manifesto.

Ferramentas nativas, como o editor de arquivos, shell ou mecanismo de patch de
cada agente, continuarão específicas do provider. Capacidades que precisem se
comportar da mesma maneira em Claude, Codex, Grok e agentes do OpenRouter serão
expostas como ferramentas MCP gerenciadas pelo Workbench. O gateway registrará
chamadas, aplicará políticas de aprovação, removerá segredos dos logs e isolará
sessões.

## Autenticação e cobrança

Os agentes nativos utilizarão as assinaturas existentes; o OpenRouter será uma opção explícita por API:

| Agente ou fornecedor | Forma de autenticação | Cobrança |
|---|---|---|
| Claude | Login oficial do Claude Code | Controlada pelo provider; o uso atual de `claude -p`, Agent SDK e aplicações terceiras consome os limites da assinatura |
| Codex | Login do Codex com ChatGPT | ChatGPT Pro |
| Grok | Login por navegador ou dispositivo do Grok Build | SuperGrok Heavy |
| OpenRouter | Chave de API armazenada no keychain do sistema | Créditos do OpenRouter, cobrados por uso |

As credenciais permanecerão sob responsabilidade das CLIs dos fornecedores ou
do keychain do sistema operacional e não poderão ser copiadas para arquivos de
workflow ou para o banco de sessões. A Anthropic controla elegibilidade e
cobrança da assinatura; o Workbench não oferece login do Claude nem promete
que um plano cobre uso programático. A interface distinguirá rotas por
assinatura do uso por API e mostrará o consumo e custo do OpenRouter por etapa.
A Anthropic pausou em 15 de junho de 2026 o crédito separado anunciado para o
Agent SDK; consulte a
[orientação atual do plano](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
antes do uso.

O terminal do Workbench derivado do Grok não reutilizará nem acessará o
armazenamento de credenciais da assinatura Grok. Somente o processo oficial e
não modificado do provider `grok` autenticará no SuperGrok.

As sessões nativas de agentes externos do VS Code e as extensões oficiais dos fornecedores representam formas diferentes de cobrança. O Workbench utilizará por padrão os fluxos oficiais de autenticação do Claude Code, Codex e Grok Build para não substituir silenciosamente uma assinatura existente por cobrança do GitHub Copilot. Sessões via Copilot e modelos BYOK do VS Code continuarão disponíveis quando selecionados explicitamente.

## Portabilidade entre editores

O VS Code será a primeira interface, não a fundação do produto. Workflows, sessões, adaptadores, regras e artefatos pertencerão ao núcleo Rust e continuarão utilizáveis sem um editor.

- **VS Code:** utilizará a extensão própria e a ponte local exposta por `workbench daemon`.
- **Terminal:** utilizará o pager do Workbench derivado do Grok por meio de
  `workbench agent stdio`, sem depender de editor.
- **CI e scripts:** utilizarão a CLI headless e a saída estruturada de eventos.
- **Zed:** conectará ao endpoint opcional de compatibilidade `workbench serve-acp`.
- **JetBrains:** utilizará seu cliente ACP e o mesmo endpoint de compatibilidade.

Recursos específicos, como apresentação de diffs, painéis, worktrees e diálogos de permissão, poderão ter aparências diferentes. A negociação de capacidades fornecerá degradação segura, enquanto os artefatos Markdown e o log de eventos permanecerão como fontes portáteis da verdade. Nenhuma lógica de workflow, fornecedor, credencial ou política poderá ser implementada dentro da extensão do VS Code.

## Segurança e controle de alterações

- Por padrão, apenas uma etapa do workflow poderá escrever na árvore de trabalho por vez.
- Revisores somente leitura não poderão modificar arquivos sem autorização do workflow.
- Comandos, edições, aprovações e transferências serão registrados como eventos.
- Ações destrutivas no sistema de arquivos, publicações externas e alterações em produção exigirão aprovação explícita.
- Segredos serão removidos dos logs e nunca armazenados nos artefatos.
- O cancelamento será propagado aos processos dos fornecedores sem perder a sessão retomável.

## Estratégia de performance

O VS Code possui um custo básico de recursos superior ao de um editor nativo, mas elimina a necessidade de construir um editor, uma interface de sessões agênticas, um cliente Git, um depurador, uma interface de testes e um renderizador de Markdown. A extensão será ativada sob demanda, reutilizará a interface nativa do VS Code quando possível e evitará um processo permanente de webview. O daemon Rust e os processos dos fornecedores serão iniciados somente quando necessários.

Limites de concorrência, buffers de eventos limitados, atualizações incrementais de artefatos e verificações de integridade impedirão que agentes inativos consumam recursos indefinidamente. Os testes de aceitação medirão a ativação da extensão, memória ociosa, responsividade do streaming de eventos e o workflow multiagente completo, não apenas a inicialização do editor.

O caminho de terminal evitará recriar um framework de terminal maduro.
Reutilizaremos a entrada, renderização, scrollback e comportamento PTY do pager
do Grok, mantendo o daemon e os providers em processos separados. Os testes de
performance do terminal cobrirão inicialização, memória, streaming de alto
volume, latência de cancelamento e reconexão.

## Estratégia open source

O mecanismo de orquestração permanecerá independente de qualquer editor ou fornecedor de modelos. Um protocolo local versionado será a fronteira principal do cliente VS Code, enquanto o ACP continuará sendo uma fronteira de interoperabilidade para editores e agentes compatíveis. Ambos terminarão em adaptadores sobre os mesmos serviços da aplicação em Rust.

A extensão do VS Code utilizará APIs públicas estáveis e continuará substituível de forma independente. O MVP não dependerá de APIs privadas ou propostas do VS Code para comportamentos centrais do workflow.

O [Grok Build](https://github.com/xai-org/grok-build) utiliza a licença
Apache-2.0 e fornece o pager de terminal usado como fundação da TUI do
Workbench. O
[fork do Workbench](https://github.com/RenatoTadeuFigueiredo/grok-build)
seguirá um modelo upstream-first com patch stack:

- `main` será um espelho somente fast-forward de `xai-org/grok-build:main`;
- `workbench` conterá um backend ACP externo pequeno e revisado;
- branches de produto nascerão de e voltarão para `workbench`;
- atualizações upstream serão aplicadas por rebase em uma branch temporária de
  sincronização, revisadas com `git range-diff` e nunca terão merge automático;
- tags de release e commits upstream testados serão imutáveis.

O patch preservará o backend original do Grok e reutilizará sua arquitetura de
ações, efeitos, renderização, scrollback, permissões, tarefas e testes. A lógica
de workflows não poderá ser implementada no fork. A decisão completa, limites
do patch, gates de compatibilidade e política de rollback estão documentados em
[`docs/architecture/grok-build-terminal-integration.md`](docs/architecture/grok-build-terminal-integration.md).

A Feature 004 implementa o subconjunto necessário do cliente ACP v1 como um
pequeno adaptador JSON-RPC/NDJSON limitado. As Features 011–012 entregam
`workbench agent stdio` via `workbench-acp-server`. O OpenRouter integra-se
pela API HTTP em Rust (`workbench-openrouter`) com orçamentos locais e ledger
de gasto por sessão—nunca como runtime de agente dentro da extensão do VS
Code. O
[SDK do ACP para Rust](https://github.com/agentclientprotocol/rust-sdk)
permanece candidato a superfícies ACP mais amplas, sujeito a revisão de
compatibilidade e supply chain.

Este repositório utiliza a [Licença Apache 2.0](LICENSE). Componentes de terceiros reutilizados ou modificados deverão preservar os respectivos arquivos de copyright, licença e avisos.

## Escopo do MVP

O MVP do monorepo (Features 001–016) inclui:

1. Adaptadores Claude, Codex e Grok com autenticação controlada pelos providers.
2. Adaptador OpenRouter Chat Completions com controles de capacidade e custo
   mais ledger de gasto por sessão.
3. Workflows multiagente configuráveis (especificação, revisão, implementação,
   validação).
4. Sessões persistentes criptografadas com pausa, retomada, cancelamento e
   reconciliação humana após resultados incertos.
5. Extensão fina do VS Code para prompts, progresso do workflow, artefatos,
   aprovações, superfícies de roteamento/etapas e controle de sessões.
6. Ponte ACP `workbench agent stdio` anexada ao daemon, CLI headless JSON e
   contrato de launch `WorkbenchBackend` para a TUI derivada do Grok.
7. Artefatos Markdown, diagramas Mermaid e aprovações configuráveis.
8. Resolução central de instruções, ciclo de vida dos MCPs (stdio e HTTPS fora
   de loopback), permissões de ferramentas, escritas nativas sob política e
   políticas de aprovação.
9. Configuração em camadas, roteamento de intenções explicável, aliases de
   modelos, descoberta de capacidades e remoção segura de providers.
10. Harnesses automatizados de adaptadores, workflows, inventário de bindings
    de aceitação, recuperação e ponta a ponta offline.

Residual fora deste monorepo: rebase dual-upstream completo do pager Grok,
suíte PTY e pin publicado do fork (`GROK_BUILD_FORK_COMPATIBILITY_PIN` fica
vazio até o fork publicar um pin). Branches paralelas, workers remotos, editor
visual de workflows, colaboração em equipe, analytics, integração mais
profunda com a janela Agents do VS Code e extensões de outros editores
permanecem roadmap futuro.

## Validações realizadas

- O workspace Rust multi-crate compila um daemon local para o mesmo usuário e
  uma CLI headless com isolamento criptografado por workspace, execução falsa
  determinística, adaptadores supervisionados (Grok ACP, Claude, Codex,
  OpenRouter), MCP central, workflows e ACP agent stdio.
- Payloads sensíveis de sessões são criptografados no SQLite; root keys usam
  Keychain no macOS ou Secret Service no Linux; exportações usam age; a
  Feature 014 adiciona ledger de gasto por sessão.
- Features 001–016 embarcam harnesses de aceitação offline em
  `workbench-testkit` (`make test-acceptance`). O inventário feature↔harness do
  repositório é gateado por `make test-acceptance-bindings` (issue #28).
- O `verifyHealth` do Speckit permanece 0 porque o registro de executáveis é
  local ao binário (ADR-0020) e não carrega runners Rust externos; os gates
  offline autoritativos são os harnesses do repositório.
- O CI padrão é offline e sem quota. Smokes live de providers e cobertura real
  de Keychain/Secret Service são opt-in / ignorados
  (`make test-platform` para credential stores).
- O caminho de produção do Grok lança `grok agent --no-leader stdio`, desativa
  auto-update, fixa digest, negocia ACP v1 e mantém credenciais fora do
  Workbench. Escritas nativas Claude/Codex exigem allowlist
  `policies.provider_native_writes` + modo approval-required (Feature 015;
  desabilitado por padrão).
- A ponte fina do VS Code cria, lista, seleciona, anexa e acompanha sessões no
  endpoint do workspace, incluindo roteamento de workflow e aprovações
  (Feature 009).
- `WorkbenchBackend` planeja `workbench agent stdio` com caminhos absolutos
  (Feature 016). O rebase dual-upstream do pager e a suíte PTY permanecem em
  `grok-build`; `GROK_BUILD_FORK_COMPATIBILITY_PIN` fica vazio até o fork
  publicar um pin.
- Evidência de entrega e mapeamento issue/PR vivem em
  [`docs/project/STATUS.md`](docs/project/STATUS.md).

## Critérios de sucesso

O MVP será considerado bem-sucedido quando um usuário puder enviar uma única solicitação de funcionalidade e:

- Acompanhar todas as etapas por uma única conversa do Workbench no VS Code ou sessão de terminal.
- Inspecionar e comentar as especificações renderizadas antes ou durante a execução.
- Concluir automaticamente ao menos um ciclo de implementação, revisão e correção.
- Aplicar as mesmas regras do repositório aos agentes nativos e aos acessados por API.
- Retomar o trabalho com segurança após reiniciar o editor ou ocorrer uma falha no fornecedor.
- Distinguir o consumo das assinaturas do custo da API do OpenRouter em cada etapa.
- Abrir a mesma sessão persistida pelo VS Code e pela interface de terminal.
- Substituir o modelo de um papel sem alterar o workflow e explicar cada
  decisão automática de roteamento antes da execução.
- Revisar uma trilha completa de prompts, decisões, comandos, edições e resultados.

## Prontidão do projeto

| Fundação | Estado | Responsável |
|---|---|---|
| Visão do produto, arquitetura e limites do MVP | Pronto | Este README |
| Licenciamento público e política de avisos de terceiros | Pronto | `LICENSE` e `NOTICE` |
| Políticas de contribuição e relato de vulnerabilidades | Pronto | `CONTRIBUTING.md` e `SECURITY.md` |
| Instruções compartilhadas do repositório | Pronto | `AGENTS.md` |
| Codificação, quebras de linha e templates de colaboração do GitHub | Pronto | Configuração do repositório |
| Decisão de integração do terminal Grok Build e política de atualização | Pronto | `docs/architecture/grok-build-terminal-integration.md` |
| Decisão de configuração, roteamento e modularidade de providers | Pronto | `docs/architecture/configuration-routing-and-providers.md` |
| Scaffold, constituição e baseline de governança do Speckit | Pronto | `doc/arch/` |
| Núcleo, sessões criptografadas, protocolo e CLI | Entregue | Feature 001 |
| Ponte fina do VS Code e controles de workflow | Entregue | Features 002, 003, 009 |
| Provider supervisionado do Grok Build por ACP | Entregue | Feature 004 |
| Provider Claude Code (+ escritas nativas sob política) | Entregue | Features 005, 015 |
| Provider Codex (+ escritas nativas sob política) | Entregue | Features 006, 015 |
| Gateway MCP central (stdio + HTTPS TLS fora de loopback) | Entregue | Features 007, 013 |
| Executor configurável de workflows multiagente | Entregue | Feature 008 |
| Provider OpenRouter, controles de custo e ledger de gasto | Entregue | Features 010, 014 |
| Workbench ACP agent stdio (MVP + anexo ao daemon) | Entregue | Features 011, 012 |
| Superfície de launch WorkbenchBackend | Entregue | Feature 016 |
| Workspace Cargo e harnesses de aceitação determinísticos | Entregue | Crates Rust + `workbench-testkit` |
| Quickstart E2E do operador | Entregue | `docs/operations/operator-e2e-quickstart.md` |
| Rebase dual-upstream completo do pager Grok e suíte PTY | Residual (fora da árvore) | [grok-build](https://github.com/RenatoTadeuFigueiredo/grok-build) |

O workspace fixa Rust 1.95.0 e as dependências diretas. O gate padrão
`make check` é determinístico e offline; a cobertura real de Keychain/Secret
Service é executada pelo gate explícito `make test-platform` no macOS e Linux.
Known Gaps do monorepo estão vazios; ver
[`docs/project/STATUS.md`](docs/project/STATUS.md).

## Desenvolvimento orientado por especificações

Este README define a visão do produto; `doc/arch/` define os requisitos de
implementação. As Features 001–016 concluíram o workflow do Speckit até
`implement` para o plano de controle do monorepo:

```text
specify → clarify → plan → tasks → analyze → implement
```

Cada fase produz artefatos Markdown revisáveis, e `speckit validate` deve passar
antes de qualquer commit do corpus ou da implementação. Mudanças futuras de
comportamento exigem uma change request rastreada e uma feature Speckit nova ou
ativa antes do código de produto.

## Próximos passos

Em uma sessão nova, execute `make context` e use
[`docs/project/STATUS.md`](docs/project/STATUS.md) como handoff durável.

1. **Operar o plano de controle do monorepo** com o
   [quickstart E2E do operador](docs/operations/operator-e2e-quickstart.md)
   (config lock, daemon, sessão, workflow, anexo no VS Code, agent stdio,
   política de custo; offline vs live).
2. **Trabalho residual de produto (fora da árvore):** concluir o rebase
   dual-upstream do pager Grok Build, a suíte PTY e o pin de compatibilidade
   publicado do fork em
   [grok-build](https://github.com/RenatoTadeuFigueiredo/grok-build)
   (consome monorepo `WorkbenchBackend` / `workbench agent stdio`).
3. **Manutenção do monorepo:** apenas novos itens de roadmap—não há backlog
   gap-zero aberto neste repositório.

## Referências

- [Agentes no VS Code](https://code.visualstudio.com/docs/agents/overview)
- [Agentes personalizados e handoffs no VS Code](https://code.visualstudio.com/docs/agent-customization/custom-agents)
- [Subagentes no VS Code](https://code.visualstudio.com/docs/agents/subagents)
- [Agentes externos no VS Code](https://code.visualstudio.com/docs/agents/agent-types/third-party-agents)
- [API de participantes do Chat do VS Code](https://code.visualstudio.com/api/extension-guides/ai/chat)
- [Modelos e BYOK no VS Code](https://code.visualstudio.com/docs/agent-customization/language-models)
- [Markdown e Mermaid no VS Code](https://code.visualstudio.com/docs/languages/markdown)
- [Agent Client Protocol](https://agentclientprotocol.com/)
- [SDK do ACP para Rust](https://github.com/agentclientprotocol/rust-sdk)
- [Documentação do Grok Build](https://docs.x.ai/build/overview)
- [Código-fonte do Grok Build](https://github.com/xai-org/grok-build)
- [Fork do Grok Build do Workbench](https://github.com/RenatoTadeuFigueiredo/grok-build)
- [Decisão de integração do terminal Grok Build](docs/architecture/grok-build-terminal-integration.md)
- [Decisão de configuração, roteamento e modularidade de providers](docs/architecture/configuration-routing-and-providers.md)
- [API do OpenRouter](https://openrouter.ai/docs/quickstart)
- [Quickstart E2E do operador](docs/operations/operator-e2e-quickstart.md)
- [Status do projeto](docs/project/STATUS.md)
- [Claude Code para VS Code](https://code.claude.com/docs/en/ide-integrations)
- [Autenticação do Codex](https://learn.chatgpt.com/docs/auth)
- [Agentes externos do Zed](https://zed.dev/docs/ai/external-agents)
- [Suporte ACP da JetBrains](https://blog.jetbrains.com/ai/2026/02/koog-x-acp-connect-an-agent-to-your-ide-and-more/)
