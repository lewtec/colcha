# Handover: linguagem composável de configuração em Rust

## Objetivo

Uma linguagem de configuração em Rust, estilo CUE (tipo e valor na mesma língua, fecha para o mais específico), **sem o sistema de módulos**, com a config **derivada dos tipos Rust**. Além disso:

- o merge é comutativo e associativo (`a + b = c`, puro, sem in-place);
- um nó pode ser sobrescrito se o valor novo for "mais novo" (propriedade de CRDT);
- tipos nativos (`_core.File`) carregam um construtor do host.

## Decisões de design

### 1. Um `Value` e uma álgebra

Não existem duas camadas (tipada e dinâmica). O tipo Rust gera `Value` (`Foo::schema()`), o arquivo parseado também produz `Value`, e a soma é `&`/`+`. Na saída, `resolve::<Foo>()` entrega o struct via serde.

**Esclarecimento (reconcilia com a decisão 6):** o `Value` é o resultado da dobra, a visão materializada, e não o estado armazenado. O estado é o conjunto de fatos da decisão 6. `&`/`+` é a união de fatos, o schema gerado pelo derive vira política de dobra por caminho, e a unificação à la CUE (meet de constraints) acontece dentro da dobra.

### 2. Override não cabe na ordem do CUE

O meet do CUE só desce (aperta), e sobrescrever sobe. Misturar os dois no mesmo operador quebra a associatividade. A saída é separar: o constraint só aperta, e o override fica num register (LWW) com stamp `(clock lógico, id do nó)`. Relógio de parede não serve, porque o desempate precisa ser determinístico.

### 3. Validade não é garantida pelo merge

O merge converge sempre (é total), mas o estado convergido pode violar um constraint que chegou depois. A validação é propriedade do estado final e roda no `resolve`. Um merge que recusa é uma transação e exige coordenação (CALM), então não é CRDT puro. Para config, o híbrido costuma bastar: merge total para replicar, e a validação como gate na hora de promover um estado a ativo.

### 4. Regex "sempre perde"

Um write que viola o constraint perde a disputa para qualquer candidato válido. Para isso convergir com constraint mutável, o estado guarda o **conjunto de candidatos**, e o vencedor é escolhido no `resolve` com o constraint final. Se nenhum candidato passa, o `resolve` devolve `Error::Fold`; o default só vale quando não há candidato (ADR-0016).

**Regra única para um campo escalar (reconcilia as decisões 2, 3 e 4):** o estado guarda o conjunto de candidatos, e a dobra escolhe o de maior stamp entre os que satisfazem o constraint final. Um valor que viola o constraint não gera conflito, é só um candidato inválido que perde. Se nenhum candidato é válido, o `resolve` devolve erro; o default só vale quando não há candidato. O register da decisão 2 é o caso particular com um candidato. Conflito (`Flat`) só existe entre dois valores concretos válidos sem stamp que os ordene.

### 5. Tipos nativos = nome de tipo + schema de argumentos

- `_core.File` é um `Native` cujos argumentos são um `Value` comum (`{path: string, mode: *"r"|"w"}`).
- Atribuir `"/dev/null"` é unificar com esse schema (o atalho de string é uma coerção declarada pelo tipo).
- O `merge` nunca abre recurso nem faz I/O. Só o `resolve` chama `File::build(args)`, e esse é o único ponto com efeito.
- O tipo do nó é a fonte da verdade, e o construtor é a única saída.
- Parecido com o `Config` + `Registry` do mmcv, mas com validação na dobra (não só no construtor) e soma comutativa.

**Esclarecimento:** "unificar com o schema" acontece na dobra, não no merge. O merge só acumula fatos, e o conflito aparece no `resolve`, em linha com as decisões 3 e 6. Também por isso o `build` falha só no `resolve`.

### 5b. Enum e despacho

`notify: Webhook | Slack | Pager` funciona como disjunção com discriminante (`kind`), que torna os ramos mutuamente exclusivos. Se dois nós escolherem `kind` diferentes, é conflito no `resolve`, ou o `kind` vai num `Lww`.

A lógica de despacho pode ficar no Rust (a config só escolhe e parametriza handlers) ou na config (regras como dado, o que exige uma mini-linguagem de expressão). **Esta decisão ficou em aberto.**

### 6. Modelo mínimo: conjunto de fatos (decisão central)

- Estado = `BTreeSet<(caminho, rank, valor)>`.
- Merge = união de conjuntos (`∪`). As leis valem de graça e não existe lógica de merge para quebrar.
- Todo o significado vai para o `resolve`, uma função pura que agrupa por caminho e dobra os fatos com a política do tipo daquele campo (meet, união, max por stamp, tombstone).
- Delete é um fato (tombstone), override é um fato com stamp maior, constraint é um fato predicado.
- `Native` é um caminho cujo tipo tem `build`, e o efeito acontece só depois da dobra.
- **Custo:** o conjunto cresce, então precisa de compactação de fatos dominados (seguro, porque a dobra é um reticulado). A validação só acontece na dobra, não na inserção.
- Em Rust: `BTreeSet<Fact>` (ordenado, então união e hash do estado são determinísticos) e `trait Fold { fn fold(&[Fact]) -> Result<Value> }` por tipo de campo.

### 7. Coleções = caminho com identidade

- `Map<string, T>`: a chave faz parte do caminho (`servers.foo.name`), e "onde mergeia" é só o caminho.
- `list<Server>` vira um map com ordem derivada. A chave (`foo`) é a identidade, e anexar "no fim" não existe (sem id, dois nós colidem). Alternativa: o host gera um id estável (hash do conteúdo ou `(clock, node)`).
- A ordem de saída padrão é pela chave (determinística, sem metadado). Um campo `order` opcional é possível.
- Listas atômicas (um valor só, `Lww` ou `Flat`) continuam disponíveis quando não há identidade por elemento. Sequência de verdade (RGA/LSEQ) fica fora do escopo.

### 8. Normalização na borda

- O derive lê `servers: Vec<Server>` e emite "map por chave com `Server` embaixo". Quem escreve a config só usa o caminho.
- `servers.foo.name` vira o fato canônico `(servers/foo/name, ...)`. Chave inválida (fora do regex, vazia, duplicada após normalizar caixa) é rejeitada na inserção, então o conjunto só contém caminhos bem formados.
- Validação em dois momentos: forma do caminho na inserção (barata, local, determinística), valores e constraints na dobra.
- Derivar `name` da chave é trivial: o derive sabe qual é o campo-chave e o preenche na materialização, sem fato nenhum.
- **Cuidado:** a normalização precisa ser função pura do schema + entrada. Versões diferentes do tipo normalizando diferente fariam os conjuntos divergirem, então o estado deve carregar uma **versão do schema**.

**Coleções atômicas e ids (reconcilia as decisões 7 e 8):**

- Sem anotação, o derive trata todo `Vec<T>` como map por chave. Lista atômica exige uma anotação explícita no campo (por exemplo `#[atomic]`).
- Id gerado pelo host: hash de conteúdo deduplica elementos iguais, `(clock, node)` não deduplica. A escolha é por campo.
- Campo-chave (`name`): proposta é não ser gravável. O derive o preenche com a chave, e um fato em `servers/foo/name` é rejeitado na inserção por forma de caminho. Isso fecha o caso `servers.foo.name = "teste"` com valor diferente de `foo`, mas depende da resposta ao item 9 de "Em aberto".

## Vocabulário canônico (alternativa em álgebra de combinadores)

Caso se prefira a álgebra explícita em vez de fatos, as peças seriam:

- **Folhas:** `Flat<T>` (igual + igual = igual, diferente = conflito), `Max<T: Ord>`, `Set<T>` (união), `Constraint` (meet).
- **Combinadores:** `Prod`, `Map<K,V>`, `Lex(A,B)` (com isso `Lww<T> = Lex(Max<Stamp>, Flat<T>)`), `Lift<A>` (novo mínimo, o `Option`/default), `Tomb<A>` (novo máximo absorvente, o delete).

No modelo de fatos (decisão 6), isso tudo vira **políticas de dobra** por tipo de campo, interpretadas pelo caminho.

## Exemplo de referência: alerta de monitoramento

```
alert: {
  name:      string & =~"^[a-z0-9-]+$"
  threshold: float & >=0 & <=100
  severity:  *"warn" | "page" | "info"
  notify:    _core.Webhook
  tags:      Set<string>
  muted:     Lww<bool>
}
```

Três contribuições independentes (plataforma, serviço, plantão) somadas em qualquer ordem: `threshold` por meet, `tags` por união, `severity` com default sobrescrito, `muted` por LWW, `notify` unificando os argumentos do `Webhook` sem abrir nada. O único efeito é o `build` no `resolve`.

## Em aberto

1. Despacho/lógica: no Rust (handlers parametrizados) ou na config (mini-linguagem de expressão como dado)?
2. Coerção de atalho (`"/dev/null"` para `{path: ...}`): regra do tipo (sugestão) ou da DSL?
3. `build` pode falhar só no `resolve`, ou deve existir um `check` puro separado?
4. Tipos nativos registrados só no host, ou a DSL também declara novos?
5. Override por campo ou por struct inteira (custo de metadados)?
6. Política de estado inválido: bloquear deploy, cair no último válido, ou só marcar?
7. Ordem de saída de `Vec<Server>`: pela chave ou campo `order`?
8. Estratégia de compactação dos fatos dominados.

**Ajustes da revisão:** o item 3 está decidido na decisão 5 (o `build` falha só no `resolve`; um `check` puro separado fica opcional). Itens novos:

- 9. `name` derivado da chave ou independente? O que acontece quando `servers.foo.name` recebe um valor diferente de `foo`?
- 10. Atômico ou map por chave: qual é o padrão do derive e qual é a anotação?
- 11. Id gerado: hash de conteúdo (deduplica) ou `(clock, node)` (não deduplica)?

## Próximos passos sugeridos

1. Definir `Fact`, `Path` e `Stamp` e implementar a união em `BTreeSet`.
2. Escrever `trait Fold` e as políticas: `Meet`, `Union`, `Lww`, `Flat`, `Tomb`.
3. Testes de propriedade (`proptest`): comutatividade, associatividade, idempotência da união e convergência de `resolve` sob qualquer ordem.
4. `#[derive(Schema)]` gerando as políticas e a normalização de caminhos.
5. `Deserializer` do serde sobre o resultado da dobra e registro de `Native` com `build`.
