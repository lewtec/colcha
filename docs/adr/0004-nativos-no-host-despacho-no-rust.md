# ADR-0004: Nativos no host, despacho no Rust

Status: accepted
Date: 2026-11-16

## Context

Um tipo nativo como `_core.File` precisa de schema de argumentos e de um construtor. A config só escolhe e parametriza. O HANDOFF deixou em aberto se a DSL declara nativos e se o despacho vira dado.

## Decision

`Native` é registrado pelo host em `NativeRegistry`. A DSL só referencia nomes permitidos pelo `Node`. `NativeHost::build` é o único ponto de efeito e roda só no `resolve`. O despacho de enum fica no Rust: a config escolhe o `kind`, o host executa. O discriminante é `Flat`; dois `kind` válidos diferentes são `Error::Conflict`.

## Consequences

A crate não precisa de uma segunda linguagem de schema. O host responde por I/O e por sandbox. O `resolve` é o único ponto de efeito controlado.

## Rejected

- A DSL declarou tipos nativos. Exigiria construção de schema dentro da config e um registro dinâmico.
- Um registro global foi usado. O host perde o controle do ciclo de vida e dos testes.
- A lógica de despacho virou dado na config. Exigiria uma mini-linguagem de expressão e um avaliador.
- O `kind` foi tratado como `Lww` na v1. Escolher um handler por stamp esconde a disputa de configuração.
