# ADR-0015: parser único com modo concreto

Status: accepted
Date: 2026-11-17

## Context

`parse` e `evaluate` tinham parsers separados. `parse.rs` mantinha um parser
concreto de cerca de 650 linhas. `eval.rs` mantinha outro parser para autoria.
Os dois repetiam o cursor, `skip_trivia`, números, strings, escapes unicode,
identificadores, chaves de objeto e a precedência de `&` e `|`. A duplicação
fazia cada mudança de gramática custar duas alterações e dois conjuntos de
testes.

## Decision

`parse` e `evaluate` compartilham o mesmo parser. O parser tem dois modos:

- modo de autoria, usado por `evaluate`: aceita referências, interpolação,
  condicionais, comprehensions e chaves com template;
- modo concreto, usado por `parse`: rejeita essas construções com
  `Error::Parse` antes de produzir fatos.

O modo concreto também rejeita chave duplicada no mesmo bloco com
`Error::Path`, preservando o contrato de erro do `parse`.

Os dois modos usam o mesmo `Collector` e o mesmo `Emitter`. `Collector` só
detecta duplicata por bloco no modo concreto; no modo de autoria a duplicata
ativa continua sendo `Error::Eval`. `Emitter` continua sendo a única camada que
valida o schema do caminho e o valor emitido.

## Consequences

- `parse.rs` deixa de existir. `parse` é um modo de `eval.rs`.
- A gramática é uma só. O modo concreto aceita o encadeamento de campos
  aninhados (`a: b: 1`) e constraints dentro de listas, que o parser antigo
  rejeitava. Construções de autoria continuam rejeitadas no modo concreto.
- Mensagens de erro de campos e valores passam a vir do parser compartilhado.
  As variantes `Error::Parse` e `Error::Path` não mudam.
- `MAX_INPUT`, `MAX_DEPTH`, texto sobrando e as variantes de erro permanecem.

## Rejected

- Manter dois parsers sincronizados. Cada recurso novo de gramática precisaria
  ser implementado duas vezes.
- Fazer `parse` chamar `evaluate` com um `Context` vazio. Referências e
  interpolação passariam pela fase de resolução e falhariam como `Error::Eval`,
  não como erro de sintaxe concreta.
