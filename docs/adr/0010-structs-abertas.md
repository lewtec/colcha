# ADR-0010: campo rest para structs abertas

Status: accepted
Date: 2026-11-17

## Context

Structs derivadas são fechadas: `lookup` e a dobra rejeitam chave desconhecida.
O modot usa definições abertas em `#PlaceConfig` e `#gtkSettingsShared`, ambas
terminadas em `...`. Um avaliador que alimente a colcha com esse config precisa
guardar as chaves que o tipo Rust não declara.

A colcha não tem como inventar campos Rust. Sem um destino explícito, aceitar
chave desconhecida perderia o dado na materialização.

## Decision

`#[colcha(rest)]` em um campo `BTreeMap<String, V>` ou `HashMap<String, V>`
abre a struct. Chaves não declaradas viram fatos no caminho do objeto e são
dobradas pelo nó de `V`. O valor dobrado mantém as chaves extras no nível do
objeto, não aninhadas no segmento do campo rest.

A materialização reúne as chaves que não são declaradas no mapa do campo rest.
O segmento do campo rest é reservado: escrita nesse caminho é `Error::Path` e a
chave literal homônima é rejeitada. Um struct tem no máximo um campo rest e uma
variante de união tagueada não pode declará-lo.

## Consequences

O campo rest é derivado, como o campo-chave: o caminho canônico de uma chave
extra é o próprio segmento. Isso mantém a forma do `Value` igual à do DSL e faz
listas atômicas de structs abertas funcionarem, porque a materialização lê o
mesmo mapa plano.

A política de merge das extras é a do nó de `V`: LWW por chave, com `Raw`, ou
estrutura por campo. O default natural do campo rest é `{}` e não aparece no
`default_value` da struct, porque o valor dobrado omite o segmento rest.

O topo de `~/modot.cue`, que adiciona campos ao schema, continua fora de escopo:
a colcha tem schema estático.

## Rejected

- Struct aberta implícita em todo tipo. Quebra o fail-closed e esconde erro de
  digitação.
- Ignorar chave desconhecida. Perde dados na materialização.
- Aninhar as extras no caminho do campo rest. Cria dois caminhos para a mesma
  chave e quebra o `Value` plano da lista atômica.
- `#[colcha(open)]` sem campo Rust. Não há onde guardar o valor materializado.
- Rest em variante de união. O payload já é um objeto literal; aceitar extras
  ali adiciona um segundo formato sem necessidade.
