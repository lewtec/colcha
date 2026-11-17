# ADR-0011: listas concat

Status: accepted
Date: 2026-11-17

## Context

O modot tem listas cuja ordem importa e cujos itens vêm de camadas diferentes:
`backup.actions`, `lint.tools.<name>.cmd` e `hosts.<name>.ips`. A colcha só
tinha lista por chave e lista atômica. A lista por chave perde a ordem de
entrada; a lista atômica usa LWW e perde a contribuição das outras camadas.

`BTreeSet` usa união, mas ordena e deduplica. Isso não serve para `cmd`, onde a
ordem é semântica, nem para itens repetidos.

## Decision

`#[colcha(concat)]` em campo `Vec<T>` ou `Option<Vec<T>>` usa
`FoldPolicy::Concat`. A dobra coleta os fatos `Candidate` de lista no caminho,
ordena por `(Stamp, Value)` e concatena os itens. Só usa os fatos `Default`
quando não há candidato. Fato não-lista no caminho é `Error::Fold`.

A ordenação por `(Stamp, Value)` torna o resultado independente da ordem de
merge. Duplicatas são preservadas: dois itens iguais são duas entradas. O merge
continua sendo união de fatos, então a operação permanece comutativa,
associativa e idempotente.

Sem candidatos, a dobra usa o default de config e depois `Node.default`. Um
campo `Option<Vec<T>>` sem fatos e sem candidato dobra para `Null`.

`min_len` continua valendo: a constraint é avaliada sobre a lista concatenada,
não sobre cada contribuição.

## Consequences

Uma camada contribui com uma lista inteira. O caso comum, uma camada por
arquivo, funciona sem estado adicional. Uma contribuição não pode ser editada
item a item: para retirar um item, o autor reescreve a contribuição com `Stamp`
maior ou aplica um tombstone no caminho.

O `Value` dobrado é plano, igual ao de uma lista atômica, então a materialização
de `Vec<T>` não muda. Tags de união dentro da lista seguem validadas pelo parser
e pelos mesmos defaults da lista atômica.

## Rejected

- União ordenada por item. Deduplica e perde itens repetidos; `bins` e `cmd`
  precisam de repetição.
- Lista CRDT com identidade de elemento. Resolve a retração por item, mas
  adiciona identidade ao fato e muda o modelo de merge. Fica para depois da v1.
- Concatenação automática em todo `Vec<T>` sem anotação. Muda o comportamento
  atual da lista por chave e esconde a escolha de política.
- Retração por item na v1. Exige tombstones por elemento e um identificador que
  o autor da config não tem.
