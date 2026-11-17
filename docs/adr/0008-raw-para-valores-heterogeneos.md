# ADR-0008: `Raw` cobre valores heterogêneos

Status: accepted
Date: 2026-11-17

## Context

`file.<profile>.<path>.values`, `options` e `desktop.raw` carregam JSON
arbitrário. O schema Rust não modela essas formas. `Value` não implementava
`NodeOf` nem `Materialize`, então `BTreeMap<String, Value>` não compilava. O
parser também tratava `{ ... }` sempre como bloco de schema.

## Decision

`Raw` é um newtype público sobre `Value`. Ele implementa `NodeOf` com
`NodeKind::Raw` e `Materialize`. O nó não tem constraint de tipo e usa
`FoldPolicy::Lww` no valor inteiro. O parser aceita objeto literal em campo
`Raw`, tanto em `raw: { ... }` quanto em `raw { ... }`. Um nó `Raw` não tem
caminhos filhos. A materialização rejeita valor nativo, inclusive aninhado.

## Consequences

`BTreeMap<String, Raw>` e `Option<Raw>` compilam. O host valida a forma do valor
depois da dobra, pelo campo `type` do arquivo. A dobra de um `Raw` é atômica:
duas contribuições substituem o valor inteiro por LWW. Isso é suficiente para
`values` e `options`, mas não faz merge por chave dentro do objeto.

## Rejected

- Implementar `NodeOf` e `Materialize` para `Value`. Expõe o tipo interno e não
  marca a intenção de valor opaco.
- Tratar `Map<String, Value>` como caso especial. Duplica a regra e não cobre
  `Option<Raw>` nem listas.
- Modelar um AST JSON com merge por chave. A semântica de união dentro de `Raw`
  não está definida e o host já valida a forma.
- Abrir todas as structs para chaves extras. Muda a validação de todo o schema.
