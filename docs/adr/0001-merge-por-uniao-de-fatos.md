# ADR-0001: Merge é união de fatos, não meet de CUE

Status: accepted
Date: 2026-11-16

## Context

O projeto precisa de merge comutativo, associativo e idempotente. Override sobe; o meet de CUE só desce. Misturar os dois no mesmo operador quebra a associatividade. O estado também precisa sobreviver a um constraint que chega depois.

## Decision

`State` é um `BTreeSet<Fact>`. `State::union` é união de conjuntos. Todo o significado fica na dobra: `resolve` agrupa fatos por caminho, aplica a política do campo, faz meet das constraints e escolhe o candidato válido.

## Consequences

As leis de merge valem sem lógica condicional. O estado cresce e precisa de compactação. Um estado convergido pode violar um constraint; a validação é gate da dobra e da promoção, não do merge.

## Rejected

- O meet de CUE foi usado como merge. Override não caberia e a associatividade quebraria.
- `&` e `+` foram mantidos como operadores separados com semânticas misturadas. O resultado não era uma álgebra.
- Duas camadas, tipada e dinâmica, foram descartadas. O tipo Rust gera `Node`, o parser gera fatos e a mesma dobra trata os dois.
