# Auditoria: qualidade dos erros

Status: auditoria
Date: 2026-11-17

## Escopo

Esta auditoria cobre a superfície de erros da colcha após a unificação dos
parsers. Nenhum código foi alterado.

Método: um exemplo temporário imprimiu `Debug` e `Display` de falhas
representativas. O exemplo foi removido depois da coleta.

## O que funciona

- As variantes são estáveis e casáveis.
- `Display` é informativo, mas não é contrato de máquina.
- Erros de sintaxe trazem linha e coluna.
- Erros semânticos trazem o caminho exato.
- `Build` encadeia a causa via `source`.
- Não há dump de unificação nem múltiplas interpretações concorrentes.

As variantes são `Parse`, `Path`, `Eval`, `Fold`, `Conflict`, `Native`, `Build`
e `Materialize`.

## Achados

### E-01: candidato inválido cai no default (alta)

Entrada:

```text
threshold: 200.0 & <=100
```

Resultado: nenhum erro. O campo dobra para `0.0`, o default natural.

O candidato explícito é descartado em silêncio. O SPEC AS-02 registra essa
escolha. O efeito é que um valor errado vira um valor plausível.

### E-02: fold não diz esperado versus encontrado (alta)

Entrada: `name: 1` em um campo `String`.

Saída:

```text
fold failed at name: no valid candidate and no valid default
```

Falta o tipo esperado, o valor encontrado e a constraint que rejeitou.

### E-03: referência ausente não é nomeada (média)

Entrada: `a: host`, sem `host` no documento nem no `Context`.

Saída:

```text
evaluation failed at a: expression does not resolve to a concrete value
```

O nome `host` não aparece.

### E-04: campo obrigatório ausente aponta para a raiz (média)

Saída:

```text
fold failed at <root>: required field has no facts and no default
```

O campo `name` não aparece.

### E-05: erro semântico não tem linha e coluna (média)

Exemplo:

```text
invalid path nmae: unknown field 'nmae'
```

O caminho `nmae` está correto. Falta a posição no arquivo.

### E-06: prefixo e jargão enganosos (baixa)

Exemplos:

```text
invalid path name.first: 'first' is below a scalar leaf
invalid path nmae: unknown field 'nmae'
```

O caminho não é inválido. A operação é. `scalar leaf` é termo interno.

Falta uma sugestão de campo próximo.

### E-07: conflito usa `Debug` (baixa)

```text
conflict at name: [String("x"), String("y")]
```

Uma forma de config seria mais legível: `"x"` versus `"y"`.

### E-08: nativo não lista os nomes permitidos (baixa)

```text
native Nope is not usable at threshold
```

Falta a lista permitida pelo `Node`.

## Comparação com CUE

O CUE mostra posição, tipo esperado e várias interpretações. O custo é ruído e
mensagens longas.

A colcha mostra uma falha por vez, com caminho e variante. O custo é menos
sinal. O achado E-01 é o caso em que a colcha perde para o CUE.

## Prioridades

- P0: falhar quando existe candidato explícito inválido. Usar default apenas
  quando não há candidato. Exige mudança de SPEC e ADR.
- P1: nomear campo, referência e nativo. Mostrar esperado versus encontrado.
  Formatar conflito como config.
- P2: propagar linha e coluna até erros semânticos. Exige span no AST ou um
  `Error::At`.

## Situação

Nenhuma correção foi aplicada. Este documento registra os achados para decisão
posterior.
