# Runbook do provedor Codex

Guia do operador (PT-BR) para o adaptador supervisionado do Codex CLI.

## Propriedade

- Autenticação e elegibilidade da assinatura ChatGPT são controladas pelo CLI
  oficial da OpenAI e pela política da OpenAI. O Workbench nunca intermedia
  credenciais.
- O operador instala e autentica o Codex fora do Workbench (`codex login`).
- O Workbench apenas supervisiona um executável absoluto configurado de forma
  explícita.

## Configuração

```yaml
providers:
  codex:
    type: subscription-cli
    driver: codex
    executable: /caminho/absoluto/para/codex
models:
  codex-default:
    provider: codex
    runtime_model: gpt-5
```

Regenere o lock do repositório após qualquer troca de executável:

```bash
cargo run -p workbench-cli -- config lock
cargo run -p workbench-cli -- config validate
```

## Comportamento

- Protocolo fixo: `codex-exec-jsonl/1`
- Perfil de lançamento: `codex exec --json --ephemeral --sandbox read-only -C <workspace> -m <model> <prompt>`
- Pré-voo de autenticação: `codex login status` com evidência de login ChatGPT
- Proteção de cobrança: remove `OPENAI_API_KEY`, `CODEX_API_KEY` e seletores de
  base-URL / OSS do ambiente do filho
- O Workbench nunca executa `codex login`, `logout`, `update` ou instaladores
- O Workbench nunca abre arquivos de credencial em `CODEX_HOME` (ex.: `auth.json`)

## Smoke ao vivo (opt-in, sem prompt)

A suíte padrão não invoca o Codex real. Operadores podem rodar:

```bash
WORKBENCH_CODEX_EXECUTABLE=/caminho/absoluto/para/codex \
WORKBENCH_CODEX_VERSION=0.145.0 \
cargo test -p workbench-codex --test live_codex -- --ignored --nocapture
```

O smoke valida apenas versão e login ChatGPT; não inicia turno de modelo.

## Recuperação

| Sintoma | Ação |
|---|---|
| Provedor indisponível após probe de auth | `codex login` fora do Workbench; confirme status ChatGPT |
| Mismatch de digest / lock | Substitua o executável de propósito e regenere o lock |
| Protocolo ou versão incompatível | Codex CLI ≥ 0.145.0, re-lock, reinicie o daemon |
| Outcome unknown após cancel/crash | Inspecione o attempt durável; sem retry automático |

## Encerramento

O shutdown do daemon rejeita trabalho novo, termina filhos Codex ativos, escala
dentro do orçamento do provedor, drena pipes e exige reaping de todos os
processos. Filho não reaped é falha de startup/shutdown, não sucesso.
