# ADR-0003: `Vec<T>` é mapa por chave por padrão

Status: accepted
Date: 2026-11-16

## Context

Lista com append no fim não tem identidade estável. Dois nós que anexam ao mesmo tempo colidem. O HANDOFF quer caminho com identidade e ordem determinística.

## Decision

O derive trata `Vec<T>` como mapa por chave. O elemento MUST ter um campo `#[colcha(key)]` do tipo `String`. A chave vira segmento de caminho e o campo-chave é derivado na materialização; escrita nesse caminho é `Error::Path`. `Vec<T>` anotado `#[colcha(atomic)]` é um valor único com `FoldPolicy::Atomic`. A saída por chave é ordenada pelo segmento.

## Consequences

A ordem padrão é determinística e não exige metadado. A chave é a identidade e duas contribuições para a mesma chave compõem. Lista atômica continua disponível quando não há identidade por elemento.

## Rejected

- RGA e LSEQ foram incluídos. Sequência de verdade fica fora do escopo da v1.
- O índice da lista foi usado como chave. Inserir no meio desloca todos os caminhos.
- IDs gerados por hash de conteúdo foram usados. Deduplicam elementos iguais, mas adiam a decisão de identidade para o host.
- IDs gerados por `(clock, node)` foram usados. Não deduplicam e criam caminhos instáveis.
- A ordem de inserção foi usada. Não é determinística entre réplicas.
- O campo `order` foi incluído na v1. Ordem por chave cobre o caso padrão.
