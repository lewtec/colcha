# ADR-0016: candidato explícito inválido é erro

Status: accepted
Date: 2026-11-17

## Context

A dobra filtrava candidatos que violavam constraints e, quando todos os
candidatos eram inválidos, seguia para o default de config e depois para
`Node.default`. Uma entrada como `threshold: 200.0 & <=100` dobrava para `0.0`
sem erro quando o campo tinha default natural.

O efeito era um valor plausível no lugar de um valor errado. O SPEC AS-02
registrava esse descarte silencioso como risco aceito, condicionado a um canal
de avisos antes da v1.

## Decision

Um fato `FactKind::Candidate` explícito que viola as constraints do campo é
`Error::Fold`, mesmo que exista default de config ou de schema. O default só é
considerado quando não há candidato.

O mesmo vale para `FactKind::Default` explícito: se o autor escreveu um default
e ele viola as constraints, a dobra falha. `Node.default` continua sendo o
último recurso quando nenhum fato explícito existe.

Candidato inválido que concorre com candidato válido continua perdendo sem
gerar conflito. A mudança só atinge o caso em que nenhum candidato é válido.

A mensagem de erro nomeia o caminho, os valores encontrados com tipo e as
constraints esperadas.

## Consequences

- Entrada explícita inválida nunca vira default silenciosamente.
- `fold` continua puro, determinístico e sem canal de avisos.
- Configurações antigas que dependiam do fallback silencioso passam a falhar.
  É uma mudança de contrato registrada em INV-33 e no SPEC AS-02.
- `Option<T>` ausente continua sendo `None`; ausência não é candidato.
- Posições de origem para erros de dobra continuam fora de escopo: `Fact` não
  carrega span. Erros semânticos do parser e do avaliador agora carregam a linha
  e a coluna da entrada; erros de sintaxe já as tinham.

## Rejected

- Manter o descarte silencioso e adicionar avisos. O fold deixaria de ser
  fail-closed e o aviso dependeria de um consumidor que pode não existir.
- Tratar candidato inválido como conflito. Conflito é desacordo entre valores
  válidos; constraint é outra classe de erro.
- Relaxar constraints para aceitar o candidato. Isso inverteria o sentido de
  `&` e quebraria INV-08.
