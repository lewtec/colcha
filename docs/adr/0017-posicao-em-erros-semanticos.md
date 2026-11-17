# ADR-0017: posição de origem em erros semânticos

Status: accepted
Date: 2026-11-17

## Context

O parser já devolvia `Error::Parse` com linha e coluna. Erros semânticos
devolviam `Error::Path` ou `Error::Eval` só com caminho e razão. Uma entrada
como `nmae: "x"` dizia qual campo era desconhecido, mas não onde ele estava no
arquivo.

As entradas do AST não guardavam origem. O coletor e o emissor recebiam apenas o
caminho, então não havia como reconstruir a linha depois.

## Decision

Cada `Entry` guarda a linha e a coluna do início da entrada. `Decl` e `Task`
copiam essa origem. `Error::Path`, `Error::Eval` e `Error::Native` ganham um
campo `position: Option<(usize, usize)>`.

`Error::at(line, column)` preenche a posição apenas se ela ainda não existe. O
erro sobe pelas camadas do coletor e do emissor sem duplicar a posição. O
`Display` acrescenta `(at linha:coluna)` ao final.

A posição é a do início do campo, não a do token que falhou. Para apontar o
token exato seria necessário um span por nó do AST.

`Error::Fold` continua sem posição: `Fact` não carrega origem e a dobra pode
misturar fatos de vários documentos e stamps.

## Consequences

- Erros de campo desconhecido, referência não resolvida, duplicata e conflito
  de avaliação mostram linha e coluna.
- As variantes `Error::Path`, `Error::Eval` e `Error::Native` mantêm o nome e
  ganham um campo opcional. Um `match` com `..` continua compilando; um `match`
  que lista todos os campos precisa de ajuste.
- O erro de dobra e o erro de `State::with` sem entrada authored não têm
  posição.
- A posição não muda o `State`, o `fold` nem a materialização.

## Rejected

- Criar `Error::At { source }`. Isso quebraria o `match` por variante usado
  pelos chamadores e pelos testes para classificar o erro.
- Guardar span em `Fact`. O fato é replicado e comparado; a origem do documento
  não pertence ao estado.
- Embutir a posição apenas no texto da razão. A posição ficaria na `Display` e
  não no valor tipado do erro.
