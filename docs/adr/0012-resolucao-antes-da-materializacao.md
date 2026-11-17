# ADR-0012: resolução antes da materialização

Status: accepted
Date: 2026-11-17

## Context

O `modot.cue` usa referências (`palette: options.base16.palette`), condicionais
(`if runtime.hostname == ...`), comprehensions (`for name, mod in modules ...`),
interpolação, defaults não literais e schema por módulo. A colcha não os
representa: `Value` é concreto e `fold` só compara candidatos.

A primeira leitura tratava esses recursos como fora de escopo. Eles estão no
escopo do produto, mas não pertencem à álgebra de fatos. Sem uma fronteira
explícita, cada recurso novo ameaça `fold`, `State` e `Materialize`.

## Decision

O fluxo é:

```text
autoria -> resolução -> Vec<Fact> -> State -> fold -> Materialize
```

A resolução é a fase que lê autoria, módulos e o schema efetivo. Ela emite
`Fact` com `Value` concreto e um `SchemaId` único. Referências, condicionais,
comprehensions, interpolação, defaults não literais e schema por módulo
terminam nessa fase.

`fold` e `Materialize` MUST NOT ver autoria. Um `Fact` nunca carrega uma
referência nem um valor parcial. A resolução pode ser o CUE do modot, um
avaliador do host ou um avaliador embutido; a álgebra não muda.

O schema por módulo é reduzido ao schema do tipo Rust na resolução. O que o tipo
não declara vira `Raw` ou um campo rest, e todos os fatos saem com o mesmo
`SchemaId`.

## Consequences

A crate continua pequena: `fold` é puro e determinístico, e `State` é um
conjunto de fatos concretos. O avaliador pode evoluir sem tocar no contrato de
merge nem na materialização. O custo é que a crate sozinha não lê `modot.cue`;
quem a alimenta precisa resolver a autoria primeiro.

Um avaliador embutido, quando existir, entra antes de `State`. Ele não altera a
assinatura de `fold`, `resolve` ou `Materialize`.

## Rejected

- Referências como fatos. `Value` é concreto e a dobra teria de interpretar
  autoria, perdendo pureza e determinismo.
- Resolver durante a dobra. Mistura avaliação com seleção de candidato e impede
  `fold` de ser uma função pura do `State`.
- Schema dinâmico dentro de `fold`. O schema precisa estar fechado antes de
  inserir fatos; caso contrário `SchemaId` e `lookup` deixam de ser estáveis.
- Reimplementar CUE inteiro dentro da crate na v1. O avaliador externo cobre a
  autoria e a álgebra permanece o produto.
