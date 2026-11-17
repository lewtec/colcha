# ADR-0013: avaliador embutido na resolução

Status: accepted
Date: 2026-11-17

## Context

O ADR-0012 fixou a fronteira: autoria termina em `Vec<Fact>` e `fold` não
interpreta referências. A v1 ainda não tinha o avaliador; o host ou o CUE do
modot precisava resolver a autoria antes de inserir fatos. Isso deixava a crate
sem uma resposta para referências, interpolação, condicionais, comprehensions e
defaults não literais quando o host não trouxesse um avaliador.

## Decision

`evaluate::<T>(source, context, stamp)` é o avaliador embutido da fase de
resolução. Ele lê a autoria, resolve os recursos abaixo e devolve fatos
concretos com um único `SchemaId`.

```text
autoria -> evaluate -> Vec<Fact> -> State -> fold -> Materialize
```

Ordem de resolução de uma referência:

1. binding local de comprehension;
2. campo do documento, inclusive referência para frente;
3. valor do `Context` fornecido pelo host.

Sintaxe e semântica:

- Referência: caminho pontuado, case-sensitive, igual ao caminho de configuração.
- Interpolação: `"\(path)"` em string e em chave de campo; aceita só escalar.
- Condicional: `if <expr> { ... } else { ... }`, com `==`, `!=` ou truthiness.
- Comprehension: `for key, value in <expr> { ... }`; a fonte é mapa ou lista.
  Sobre mapa a chave é `String`; sobre lista o índice é `Int`.
- Default não literal: `*<expr>` resolve para um valor concreto.

O avaliador MUST emitir apenas `Fact` com `Value` concreto. `fold`, `State` e
`Materialize` não mudam. Referência não resolvida, ciclo, campo ativo duplicado
ou referência ambígua devolvem `Error::Eval`.

O `Context` é imutável para quem chama. `with` e `merge` devolvem um novo
`Context`; um valor nunca é alterado inplace, só combinado em um novo valor.
`merge` é comutativo e idempotente quando não há valores diferentes no mesmo
caminho.

## Consequences

`parse` continua sendo o front end do subconjunto concreto. `evaluate` cobre a
autoria e pode evoluir sem tocar na álgebra. O custo é uma segunda gramática
voltada a autoria, limitada de propósito: sem aritmética, `&&`, `||`, funções,
imports ou módulos. Um campo que só declara constraints não é referenciável,
porque não tem valor concreto.

## Rejected

- Avaliar durante a dobra. Mistura resolução com seleção de candidato e quebra a
  pureza de `fold`.
- Implementar CUE inteiro. Arrasta módulos, unificação completa e aritmética para
  dentro da crate.
- Substituição textual de referências. Não respeita escopo de comprehension,
  ciclos, desambiguação nem a ordem documento/contexto.
- Emitir fatos parciais com referências simbólicas. `Value` é concreto e o
  `State` não pode carregar autoria.
