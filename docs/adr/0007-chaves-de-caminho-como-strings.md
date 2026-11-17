# ADR-0007: segmentos de `Path` são strings case-sensitive

Status: accepted
Date: 2026-11-17

## Context

O config do modot usa chaves como `base0A`, `org/gnome/desktop/interface`,
`.config/legacy` e `*.txt`. A gramática antiga aceitava só
`[a-z0-9][a-z0-9_-]*` e normalizava a caixa. Isso descartava informação e
criava colisões entre chaves distintas.

## Decision

Cada segmento de `Path` é uma `String` não vazia e case-sensitive. O parser
aceita palavra nua ou literal de string. `Path::Display` usa palavra nua quando
possível e cita os demais segmentos com escapes JSON, juntando com `.`.
`Path::parse` reconstrói essa forma. O schema decide se uma chave é válida:
`Struct` só aceita campos declarados; `Map` e `List` por chave aceitam qualquer
segmento não vazio.

## Consequences

`base0A` e `base0a` são caminhos distintos. Chaves com `.`, `/`, `*` e espaços
ficam representáveis. O limite de 64 bytes por segmento deixa de existir; o
parser ainda limita a entrada total a 1 MiB e o caminho a 32 segmentos. A ordem
lexicográfica de `Path` continua total e determinística.

## Rejected

- Manter normalização de caixa. Perde informação e colide chaves.
- Usar `%`-encoding na DSL. A saída fica ilegível para chaves comuns.
- Tratar chave de mapa como categoria opaca separada. A chave é só `String`; a
  validade pertence ao nó do schema.
- Manter o limite de 64 bytes. Caminhos reais de driver e de arquivo passam
  disso.
