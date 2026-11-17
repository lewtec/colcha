# ADR-0002: DSL própria, sem sistema de módulos

Status: accepted
Date: 2026-11-16

## Context

A config precisa de uma superfície textual que expresse constraints, defaults e tipos nativos. O HANDOFF recusa o sistema de módulos do CUE. A v1 é uma crate; CLI e formatos serde ficam fora.

## Decision

A v1 aceita um subconjunto CUE-like com parser recursive descent próprio: `&`, `|`, `*`, comparações, regex, listas, objetos, `_core.Nome` e caminhos pontuados. O parser produz `Fact` normalizado. Serde fica só na materialização de saída.

## Consequences

A gramática é pequena e testável. O projeto é dono do parser, das mensagens e dos limites de entrada. Entrada JSON, TOML e YAML exige outro caminho de normalização.

## Rejected

- A engine CUE foi adotada. É Go, traz módulos e não deriva schema de tipo Rust.
- `cue-rust` foi adotado. Tem módulos, não faz decode tipado e cobre um subconjunto maior do que a v1 precisa.
- JSON, TOML e YAML foram aceitos como entrada na v1. A normalização por tipo ficaria sem uma segunda superfície de teste.
