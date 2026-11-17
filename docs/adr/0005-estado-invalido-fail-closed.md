# ADR-0005: Estado inválido é fail-closed na promoção

Status: accepted
Date: 2026-11-16

## Context

O merge converge sempre. O estado convergido pode violar um constraint que chegou depois. Recusar o merge exigiria coordenação e deixaria de ser CRDT. A pergunta é o que o host faz com o estado inválido.

## Decision

`State::union` é total e nunca recusa. `fold` e `resolve` devolvem erro quando o estado viola constraint, quando falta campo obrigatório e quando a identidade do schema não bate. O gate de promoção recusa o estado inválido; o último estado ativo válido continua servindo. Cada `Fact` carrega `SchemaId` com nome e versão do tipo raiz.

## Consequences

A replicação nunca bloqueia. A disponibilidade da config ativa fica com o host. A identidade do schema impede dobrar estado de outro tipo raiz com a mesma versão.

## Rejected

- O estado inválido caiu no último válido em silêncio. O host perderia o sinal de que a contribuição nova não vale.
- O estado inválido foi servido com aviso. O deploy passaria com config que viola constraint.
- O merge recusou estado inválido. Exigiria coordenação e não seria CRDT puro.
- A versão do schema ficou só no cabeçalho do estado. Dois tipos raiz com a mesma versão seriam indistinguíveis.
