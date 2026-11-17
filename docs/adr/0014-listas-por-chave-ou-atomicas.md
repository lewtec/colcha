# ADR-0014: listas por chave ou atômicas

Status: accepted
Date: 2026-11-17

## Context

O ADR-0011 introduziu `#[colcha(concat)]` para listas cuja ordem importa e cujos
itens vêm de camadas diferentes. A política ordena por `(Stamp, Value)` e
concatena os itens. Isso resolve `backup.actions`, mas adiciona uma terceira
política de lista e mais um uso de `Stamp` na dobra.

O modelo já tem duas formas de lista. A lista por chave usa `#[colcha(key)]` e
ordena pelo segmento da chave. A lista atômica é um valor inteiro com LWW. A
lista por chave compõe itens quando o item tem uma chave. A lista atômica cobre
a ordem do valor quando a lista é uma unidade.

## Decision

`FoldPolicy::Concat` e `#[colcha(concat)]` não existem. Toda `Vec<T>` é:

- lista por chave, com exatamente um campo `#[colcha(key)]`; ou
- lista atômica, anotada `#[colcha(atomic)]`.

A ordem da lista por chave vem da chave. A lista atômica é um valor LWW inteiro;
listas diferentes no mesmo caminho com o mesmo `Stamp` são `Error::Conflict`.

`Stamp` continua no modelo para precedência de valor e para tombstones. O
`Concat` deixa de ser um terceiro consumidor dessa ordem.

## Consequences

- `backup.actions` precisa de chave, `BTreeMap`/`Raw`, ou `atomic`. Perde a
  concatenação automática entre camadas.
- `cmd` e `ips` viram `atomic` e não aceitam contribuição parcial.
- A superfície pública encolhe: um `FoldPolicy`, um atributo e um caminho de
  dobra a menos.
- Lista ordenada sem chave volta a ser caso de `Raw` ou de um tipo com chave
  explícita.

## Rejected

- Manter `Concat`: terceira política de lista e ordenação por stamp para uma
  estrutura que o modelo de chave já cobre.
- Concat com chave explícita: é a lista por chave com outro nome.
- Ordem de chegada: quebra comutatividade.
