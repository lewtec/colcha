# ADR-0009: uniões tagueadas em enums

Status: accepted
Date: 2026-11-17

## Context

O modot usa uniões etiquetadas para ações com payload:

- `#BackupActionGitRepoSync | #BackupActionRsync | #BackupActionArchive`.
- `#PlaceStepMove | #PlaceStepRequire`.

O derive da colcha aceitava apenas variantes unitárias. A folha `Raw` representava
os valores, mas perdia validação de campos, defaults e a ligação com o tipo Rust.
O parser também tratava todo `{ ... }` como bloco de schema, então um valor de
união não tinha forma de ser escrito.

## Decision

`#[colcha(tag = "kind")]` transforma o enum em `NodeKind::Union`. O nó guarda
`tag` e `variants`. Cada variante nomeada vira um nó `Struct`; a variante unitária
vira um `Struct` vazio.

O fato é um único `Value::Map` no caminho da união. O parser exige o campo tag e
rejeita tag ausente ou desconhecida. A dobra seleciona o candidato por LWW, lê o
tag, escolhe a variante, preenche defaults dos campos, rejeita campo desconhecido
e mantém o tag no valor dobrado. A materialização lê o tag e monta a variante
Rust.

`Option<E>` aceita `null`. `Vec<E>` exige `#[colcha(atomic)]`; a materialização
aplica defaults dos campos e rejeita campo desconhecido, porque a lista atômica
não desce por caminho.

## Consequences

Enums sem tag continuam `Flat` e unitários. Uniões que misturam escalar e struct
continuam cobertas por `Raw` na v1. `#[colcha(key)]` em campo de variante e campo
com o nome do tag são erros de compilação. Caminhos abaixo da união são
`Error::Path`.

A união é atômica no merge: contribuições no mesmo caminho substituem o valor
inteiro por LWW. Merge por campo dentro de uma variante exigiria caminhos sob a
união e um tag por caminho; fica fora da v1.

## Rejected

- Enum com payload sem tag. A forma do valor não identifica a variante.
- Tag adjacente ou interno. O DSL já usa `{ ... }` como objeto literal; um campo
  explícito é mais simples de validar.
- Caminhos por variante, como `action.start.service`. Multiplica o `lookup` e
  permite estados com duas variantes ao mesmo tempo.
- Usar `Raw` para todas as uniões. Perde defaults e a validação do tipo Rust.
- Emitir a representação `serde` do enum. A colcha não depende de `serde` para o
  formato da DSL.
