# Colcha Specification

Crate Rust que compõe configuração tipada a partir de fatos replicáveis: merge por união, dobra determinística e materialização a partir do tipo.

Status: approved
Genre: library

The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY in this
document are to be interpreted as described in BCP 14 (RFC 2119,
RFC 8174) when, and only when, they appear in all capitals.

## Intention

Job:

Permitir que um programa Rust derive o schema da configuração do próprio tipo, aceite contribuições independentes como fatos, una essas contribuições em qualquer ordem e materialize um valor tipado determinístico.

Non-goals:

1. Sistema de módulos, imports e pacotes na DSL.
2. Protocolo de rede, transporte de replicação e armazenamento do estado.
3. Sequências de verdade (RGA, LSEQ, inserção no meio de lista).
4. Efeito de I/O durante merge, dobra e validação.
5. CLI na v1.
6. Entrada JSON, TOML e YAML na v1; serde fica na materialização de saída.
7. Tipos nativos declarados pela DSL.
8. Mini-linguagem de expressão para despacho na config.
9. Migração de schema entre versões na v1.
10. Gestão e redação de segredos.

Inherited C (cite the file):

- Rust como linguagem: `HANDOFF.md`.
- Modelo de estado como conjunto de fatos, merge por união, dobra no `resolve`: `HANDOFF.md`.
- Ausência de sistema de módulos: `HANDOFF.md`.
- `HANDOFF.md` não contém código; não existe `Cargo.toml` neste repo.
- Edition 2024 e MSRV 1.93 são convenção da org (`lewtec/palafita/Cargo.toml`), não C deste repo.

## Technique

| ID | Input | Rule | Output |
|----|-------|------|--------|
| TEC-01 | dois `State` | união de `BTreeSet<Fact>` | `State` novo |
| TEC-02 | `State` + `Node` | agrupar fatos por caminho, aplicar a política do campo, fazer meet das constraints, escolher o candidato válido e aplicar tombstone | `Value` concreto |
| TEC-03 | `Fact` + `Schema` | validar segmento, profundidade, caminho conhecido e escrita em campo-chave | `State` atualizado |
| TEC-04 | tipo Rust + atributos | proc-macro lê campos, chaves, políticas e versão | `Node` estático + `Schema::NAME` + `Schema::VERSION` |
| TEC-05 | texto + `Node` + `SchemaId` + `Stamp` | lexer e parser recursive descent; normalização de caminhos | `Vec<Fact>` |
| TEC-06 | `Value` + `T` + `NativeHost` | serde deserializer para campos comuns; folha nativa chama `build` | `T` |
| TEC-07 | nome nativo + args dobrados | `NativeRegistry::spec` fornece o schema de args; `NativeHost::build` roda uma vez por nó | `NativeHandle` |
| TEC-08 | escrita do host | `Stamp(clock, NodeId)` com ordem total | ordem determinística |
| TEC-09 | `Vec<T>`, `Map<K,V>` | chave vira segmento; campo-chave é derivado e não gravável | caminhos canônicos |
| TEC-10 | `State` + `Schema` | remover fatos dominados por stamp e política | `State` menor com mesmo fold |
| TEC-11 | falha de operação pública | classificar em `Error` estável | `Result<T, Error>` |
| TEC-12 | estados gerados | proptest afirma comutatividade, associatividade, idempotência e convergência | seed que falha |
| TEC-13 | enum Rust com variantes nativas | discriminante `kind` é `Flat` | variante única |
| TEC-14 | texto de entrada | limite de 1 MiB e profundidade 32 | `Error::Parse` acima do limite |
| TEC-15 | `State` e `Value` compartilhados | tipos imutáveis `Send + Sync`; merge por `&self` | merge sem lock |
| TEC-16 | padrão `=~` e valor string | compilar o padrão com `regex` e casar | `true` ou `false` |

## Tooling

| TEC | Tool | Relation | We do not | Cite |
|-----|------|----------|-----------|------|
| TEC-01 | `BTreeSet` | adopt | escrever conjunto e união | `stdlib:alloc::collections::BTreeSet` |
| TEC-02 | nenhum | implement | delegar a dobra | `none` |
| TEC-03 | nenhum | implement | delegar validação de caminho | `none` |
| TEC-04 | `syn`, `quote`, `proc-macro2` | implement | escrever um compilador de tipo | `stdlib:core::any` não lê nomes de campo |
| TEC-05 | nenhum | implement | delegar o parser | `none` |
| TEC-06 | `serde` | adopt | escrever serialização | `lewtec/palafita` |
| TEC-07 | nenhum | implement | delegar o registro de nativos | `none` |
| TEC-08 | `Ord` | adopt | escrever ordem total | `stdlib:core::cmp::Ord` |
| TEC-09 | nenhum | implement | delegar a normalização | `none` |
| TEC-10 | nenhum | implement | delegar compactação | `none` |
| TEC-11 | `std::error::Error` | adopt | escrever a trait de erro | `path:lewtec/palafita/crates/api/src/error.rs` |
| TEC-12 | `proptest` | adopt | escrever gerador e shrinker | `crates.io/proptest` |
| TEC-13 | nenhum | implement | delegar despacho de enum | `none` |
| TEC-14 | nenhum | implement | delegar limites de entrada | `none` |
| TEC-15 | `Send`, `Sync` | adopt | escrever sincronização | `stdlib:core::marker::Send` |
| TEC-16 | `=~` e valor string | `regex` compila o padrão e casa | `true` ou `false` | `crates.io/regex` |

Dependências novas aprovadas na sessão de grilling: `serde`, `proc-macro2`, `syn`, `quote`, `proptest`, `regex`. As três primeiras sustentam TEC-04; `proptest` sustenta TEC-12; `regex` sustenta TEC-16.

| Cell | Pick | C/D | Implements | Cite if C |
|------|------|--------|------------|-----------|
| Language | Rust | C | TEC-04, TEC-15 | `HANDOFF.md` |
| Edition / MSRV | 2024 / 1.93 | D | TEC-15 | `lewtec/palafita/Cargo.toml` |
| Runtime | síncrono, sem runtime async | D | TEC-15 | `none` |
| Persistence | nenhuma; `State` é serializável e o host persiste | D | TEC-01, TEC-06 | `none` |
| UI | N/A: genre=library | N/A | — | — |
| Packaging | workspace cargo `colcha` + `colcha-derive` | D | TEC-04 | `none` |
| Identity | `NodeId` fornecido pelo host | D | TEC-08 | `none` |
| Host OS | independente de SO, `std` | D | TEC-15 | `none` |

## Terminology

| Concept | Approved | Banned |
|---------|----------|--------|
| conjunto de fatos | `State` | documento, store, banco |
| contribuição atômica | fato (`Fact`) | evento, operação, patch |
| combinar estados | merge (`State::union`) | aplicar, sincronizar, resolver |
| materializar sem efeito | dobra (`fold`) | merge, eval, aplicar |
| materializar com efeito | `resolve` | load, apply, build |
| valor concorrente | candidato | valor final, escrita |
| remoção | tombstone | delete, remove, null |
| disputa entre candidatos | conflito | erro de merge, ambiguidade |
| regra de dobra por campo | política (`FoldPolicy`) | estratégia, modo |
| tipo com construtor do host | nativo (`Native`) | plugin, handler, recurso |
| identidade do schema | `SchemaId` (nome + versão) | versão do documento |
| identidade de elemento | campo-chave | id, nome |

## Types

| Type | Exported | Identity or value | Mutable | Nil/error | Callers MUST NOT |
|------|----------|------|---------|-----------|------------------|
| `Path` | sim | valor; segmentos case-sensitive | não | `Error::Path` na construção inválida | construir vazio, fundo demais ou com segmento vazio |
| `NodeId` | sim | valor; 16 bytes opacos | não | não tem nil; 16 bytes zero são válidos | assumir unicidade global sem o host |
| `Stamp` | sim | valor `(u64, NodeId)` | não | não tem nil | gerar dentro da crate; o host fornece |
| `SchemaId` | sim | valor `(String, u32)` | não | não tem nil | comparar schemas de tipos raiz diferentes como iguais |
| `Fact` | sim | valor | não | `Error::Path` na inserção | inserir sem `Schema` |
| `FactKind` | sim | valor (`Candidate`, `Default`, `Constraint`, `Tombstone`) | não | `Error::Fold` na combinação inválida | usar `Tombstone` para apagar constraint de schema |
| `Value` | sim | valor concreto | não | `Error::Fold` se não for concreto | conter `Constraint` e conflito depois da dobra |
| `Raw` | sim | `Value` opaco | não | `Error::Materialize` com nativo | usar `Raw` para ignorar o tipo declarado |
| `Float` | sim | valor `f64` com ordem total por bits | não | `Error::Fold` em NaN | comparar com o `PartialOrd` de `f64` |
| `Constraint` | sim | valor; predicado | não | `Error::Fold` se o meet for vazio | afrouxar outra constraint |
| `State` | sim | valor; `BTreeSet<Fact>` | sim, por `insert`; `union` e `compact` devolvem novo | vazio é válido | editar o `BTreeSet` fora de `insert` |
| `Node` | sim | valor; schema derivado | não | `Error::Fold` se o schema for inválido | editar depois de publicado |
| `FoldPolicy` | sim | valor (`Lww`, `Flat`, `Union`, `Atomic`, `Native`) | não | não tem nil | mudar entre chamadas |
| `Native` | sim | valor; nome + `Node` de args + coerção | não | `Error::Native` se ausente | fazer I/O |
| `NativeHandle` | sim | identidade opaca | não | `Error::Build` se o host falhar | assumir tipo concreto sem downcast |
| `NativeRegistry` | sim | identidade do host | não | `Error::Native` | chamar `build` |
| `NativeHost` | sim | identidade do host | não | `Error::Native`, `Error::Build` | chamar `build` na dobra e no merge |
| `Schema` (trait) | sim | type-level | não | `Error::Schema` | devolver schema diferente entre chamadas |
| `Materialize` (trait) | sim | type-level | não | `Error::Materialize` | fazer I/O fora de `NativeHost` |
| `Error` | sim | valor | não | não tem nil | casar por texto de mensagem |

## Invariants

| ID | Predicate | On | Forbidden bypass |
|----|-----------|----|------------------|
| INV-01 | Todo `Path` não-raiz tem 1 a 32 segmentos não vazios e case-sensitive; a raiz é o path vazio. | `Path` | `Path::from_segments` sem validação |
| INV-02 | `Stamp` ordena por `clock` e desempata por `NodeId` em ordem lexicográfica; a ordem é total. | `Stamp` | comparação que ignora `NodeId` |
| INV-03 | Todo `Fact` carrega `SchemaId` e é imutável. | `Fact` | fato sem identidade de schema |
| INV-04 | `State::union` é comutativa, associativa e idempotente, e não faz I/O. | `State` | merge com lógica condicional; merge que abre recurso |
| INV-05 | `State::insert` rejeita caminho malformado, caminho desconhecido no `Schema`, escrita em campo-chave e `SchemaId` diferente. | `State` | inserir direto no `BTreeSet` |
| INV-06 | `State::compact` devolve estado cujo `fold` é igual ao do estado original para o mesmo `Schema`. | `State` | compactar sem `Schema`; remover fato de `Union` |
| INV-07 | Depois da dobra, `Value` é concreto: não contém `Constraint`, não contém conflito e usa `BTreeMap` e `Vec`; ambos mantêm ordem determinística. | `Value` | devolver valor parcial; usar `HashMap` |
| INV-08 | Uma constraint só estreita o conjunto aceito; meet de constraints incompatíveis é `Error::Fold`. | `Constraint` | aceitar valor que viola uma constraint |
| INV-09 | `Schema::schema()` devolve o mesmo `Node`; `Schema::NAME` e `Schema::VERSION` são constantes do tipo. | `Schema` | schema dependente de ambiente |
| INV-10 | A dobra é determinística: o mesmo `State` e `Schema` produzem o mesmo `Value` serializado. | `fold` | iterar `HashMap`; usar relógio de parede |
| INV-11 | Candidato que viola constraint não gera conflito e nunca vence candidato válido. | `fold` | tratar inválido como conflito; escolher por ordem de chegada |
| INV-12 | `NativeHost::build` roda no máximo uma vez por nó nativo e só depois de dobra bem-sucedida. | `resolve` | build na dobra; build antes de validar |
| INV-13 | `resolve` e `fold` recusam `State` com `SchemaId` diferente de `T::NAME` + `T::VERSION`. | `resolve`, `fold` | dobrar schema desconhecido; ignorar identidade |
| INV-14 | Tombstone no caminho P absorve fatos em P e abaixo de P com `stamp <= tombstone.stamp`. | `fold` | apagar só o nó exato; apagar fato mais novo |
| INV-15 | Escrita em campo-chave é rejeitada na inserção; o valor do campo-chave é derivado do segmento. | `State`, `fold` | aceitar `servers/foo/name = "bar"` |
| INV-16 | `Vec<T>` sem `#[colcha(atomic)]` exige campo `#[colcha(key)]`; o derive emite erro de compilação quando falta. | derive | escolher índice; escolher hash |
| INV-17 | A saída de `Vec<T>` por chave é ordenada pelo segmento da chave. | `fold`, `Materialize` | ordem de inserção |
| INV-18 | O parser limita entrada a 1 MiB e aninhamento a 32; acima disso devolve `Error::Parse`. | parser | alocar sem limite |
| INV-19 | O discriminante de enum nativo é `Flat`: dois `kind` válidos diferentes produzem `Error::Conflict`. | `fold` | escolher o `kind` por stamp |
| INV-20 | `Error` é classificado por variante; `Display` não é contrato de máquina. | `Error` | fazer parse de mensagem |
| INV-21 | A coerção declarada por `Native` é pura e roda na dobra, nunca no merge. | `fold` | abrir recurso para descobrir o tipo |
| INV-22 | O mesmo bloco não pode declarar a mesma chave duas vezes. Chaves são case-sensitive. | parser | aceitar `Foo` e `foo` como a mesma chave |
| INV-23 | `Float` rejeita NaN e ordena pelos bits; a ordem é total e determinística. | `Float` | usar `PartialOrd` de `f64` no `BTreeSet` |
| INV-24 | `Path::Display` escreve uma forma que `Path::parse` reconstrói; segmentos fora da forma nua são citados com escapes JSON. | `Path` | escrever segmento ambíguo sem aspas |
| INV-25 | Um nó `Raw` aceita qualquer `Value` concreto e não tem filhos; a materialização rejeita nativo. | `Raw`, `Node` | aceitar `raw.campo`; aceitar `Value::Native` em `Raw` |

## Errors

| Public operation | Bad input | One reaction |
|------------------|-----------|--------------|
| `parse::<T>(source, stamp)` | sintaxe inválida | `Error::Parse` com linha e coluna; nenhum fato é devolvido |
| `parse::<T>` | caminho inválido, campo desconhecido | `Error::Path`; nenhum fato é devolvido |
| `parse::<T>` | escrita em campo-chave | `Error::Path`; nenhum fato é devolvido |
| `parse::<T>` | chave duplicada no mesmo bloco | `Error::Path`; nenhum fato é devolvido |
| `parse::<T>` | entrada acima de 1 MiB, aninhamento acima de 32 | `Error::Parse`; nenhum fato é devolvido |
| `parse::<T>` | nativo fora do conjunto permitido pelo `Node` | `Error::Native`; nenhum fato é devolvido |
| `State::insert::<T>(fact)` | caminho malformado, caminho desconhecido, escrita em campo-chave | `Error::Path`; estado inalterado |
| `State::insert::<T>` | `SchemaId` diferente de `T::NAME` + `T::VERSION` | `Error::Schema`; estado inalterado |
| `State::insert::<T>` | fato já presente | sem erro; o conjunto não muda |
| `State::union(other)` | qualquer estado | sem erro; união total |
| `State::compact::<T>()` | fato de caminho desconhecido, fato de outro `SchemaId` | sem erro; o fato é preservado |
| `fold::<T>(state, registry)` | candidato viola constraint e não há default | `Error::Fold` com caminho e razão |
| `fold::<T>` | dois candidatos válidos incompatíveis | `Error::Conflict` com caminho e candidatos |
| `fold::<T>` | campo obrigatório ausente sem default | `Error::Fold` com caminho |
| `fold::<T>` | `SchemaId` diferente de `T::NAME` + `T::VERSION` | `Error::Schema` com esperado e encontrado |
| `fold::<T>` | nativo sem `Native` no registry | `Error::Native` com nome e caminho |
| `resolve::<T>(state, host)` | build de nativo falha | `Error::Build` com nome, caminho e fonte |
| `resolve::<T>` | `Value` não materializa em `T` | `Error::Materialize` com caminho |
| `resolve::<T>` | dobra falha | propaga `Error::Fold`, `Error::Conflict`, `Error::Schema`, `Error::Native`; não chama `build` |

## Public contract

A crate `colcha` expõe a álgebra. A crate `colcha-derive` implementa `#[derive(Schema)]` e `#[derive(Materialize)]`. `colcha` reexporta os dois derives.

```rust
pub fn parse<T: Schema>(source: &str, stamp: Stamp) -> Result<Vec<Fact>>;
pub fn fold<T: Schema>(state: &State, registry: &dyn NativeRegistry) -> Result<Value>;
pub fn resolve<T: Schema + Materialize>(state: &State, host: &dyn NativeHost) -> Result<T>;

impl State {
    pub fn empty() -> State;
    pub fn insert<T: Schema>(&mut self, fact: Fact) -> Result<()>;
    pub fn union(&self, other: &State) -> State;
    pub fn compact<T: Schema>(&self) -> State;
    pub fn facts(&self) -> impl Iterator<Item = &Fact>;
}
```

`parse` usa `T::schema()`, `T::NAME` e `T::VERSION`. `fold` é a verificação pura. `resolve` executa `fold`, valida a identidade do schema e materializa; só `resolve` chama `NativeHost::build`.

### Core shapes

```rust
pub struct Fact {
    pub path: Path,
    pub stamp: Stamp,
    pub schema: SchemaId,
    pub kind: FactKind,
}

pub enum FactKind {
    Candidate(Value),
    Default(Value),
    Constraint(Constraint),
    Tombstone,
}

pub struct Stamp { pub clock: u64, pub node: NodeId }
pub struct SchemaId { pub name: String, pub version: u32 }

pub struct Node {
    pub kind: NodeKind,
    pub policy: FoldPolicy,
    pub constraints: Vec<Constraint>,
    pub default: Option<Value>,
    pub fields: BTreeMap<String, Node>,
    pub element: Option<Box<Node>>,
    pub key_field: Option<String>,
    pub natives: BTreeSet<String>,
    pub optional: bool,
}

pub enum NodeKind {
    Leaf,
    Raw,
    Struct,
    List,
    Map,
    Native,
}

pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(Float),
    String(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
    Native { name: String, args: Box<Value> },
}

pub struct Raw(pub Value);

pub enum Constraint {
    Type(String),
    Min(Value),
    Max(Value),
    Greater(Value),
    Less(Value),
    Regex(String),
    OneOf(Vec<Value>),
    NativeOneOf(Vec<String>),
}

pub struct Native {
    pub name: String,
    pub args: Node,
    pub coerce: Option<fn(&Value) -> Option<Value>>,
}

pub trait Schema {
    const NAME: &'static str;
    const VERSION: u32;
    fn schema() -> Node;
}

pub trait Materialize: Sized {
    fn materialize(value: &Value, host: &dyn NativeHost, path: &Path) -> Result<Self>;
}

pub trait NativeRegistry {
    fn spec(&self, name: &str) -> Option<&Native>;
}

pub trait NativeHost: NativeRegistry {
    fn build(
        &self,
        name: &str,
        args: &Value,
    ) -> Result<NativeHandle, Box<dyn std::error::Error + Send + Sync>>;
}

pub trait NativeType: Sized {
    const NAME: &'static str;
    fn from_native(handle: NativeHandle) -> Result<Self, String>;
}
```

`NativeHandle` guarda um valor opaco e oferece `downcast::<T>()`. O derive usa `NativeType::NAME` em `#[colcha(native)]` e `NativeType::from_native` na materialização.

`Schema::NAME` MUST ser o caminho qualificado do tipo: `module_path!()` mais o nome do tipo. Dois tipos homônimos em crates diferentes não podem compartilhar `SchemaId`.

`Float` é um newtype sobre `f64` com ordem total pelos bits. NaN é rejeitado na dobra com `Error::Fold`; o `State` nunca contém NaN.

### Error classes

| Variant | Payload | Raised by |
|---------|---------|-----------|
| `Error::Path` | caminho, razão | parser e `State::insert` |
| `Error::Parse` | linha, coluna, mensagem | parser |
| `Error::Schema` | esperado `SchemaId`, encontrado `SchemaId` | `State::insert`, `fold`, `resolve` |
| `Error::Fold` | caminho, razão | `fold`, `resolve` |
| `Error::Conflict` | caminho, candidatos | `fold`, `resolve` |
| `Error::Native` | caminho, nome | parser, `fold`, `resolve` |
| `Error::Build` | caminho, nome, fonte | `resolve` |
| `Error::Materialize` | caminho, mensagem | `resolve` |

### DSL surface (v1)

O parser MUST aceitar este subconjunto. A gramática é a autoridade; o exemplo no fim desta seção é informativo.

```ebnf
config   = { field } ;
field    = path ":" expr | path "{" config "}" ;
path     = segment { "." segment } ;
expr     = term { ( "&" | "|" ) term } ;
term     = "*" atom
         | ">=" number | "<=" number | ">" number | "<" number
         | "=~" string
         | "float" | "int" | "string" | "bool"
         | atom
         | native ;
atom     = string | number | "true" | "false" | "null"
         | "[" [ expr { "," expr } ] "]"
         | "{" config "}" ;
native   = "_core." ident [ "{" config "}" ] ;
ident    = ( "A".."Z" | "a".."z" | "_" ) { "A".."Z" | "a".."z" | "0".."9" | "_" } ;
segment  = bare | string ;
bare     = ( "A".."Z" | "a".."z" | "0".."9" | "_" )
           { "A".."Z" | "a".."z" | "0".."9" | "_" | "-" } ;
```

- `number` e `string` seguem JSON. `ident` nomeia o tipo nativo.
- `&` é meet de constraints. `|` é disjunção. `&` liga mais forte que `|`; `a & b | c` é `(a & b) | c`. `*` marca o default.
- `//` inicia comentário de linha. Strings usam aspas duplas e escapes JSON.
- `a.b.c: v` equivale a `a: { b: { c: v } }`.
- Um segmento é uma palavra nua ou uma string entre aspas. A string aceita qualquer caractere, incluindo `.`, `/`, `*` e maiúsculas.
- Chaves são case-sensitive: `base0A` e `base0a` são segmentos diferentes.
- Em campo `Raw`, `{ ... }` é um objeto literal e não um bloco de schema. O objeto aceita chaves e valores arbitrários.
- `_core.Nome` fixa o tipo nativo do caminho. Em campo de tipo nativo único, o prefixo MAY ser omitido.
- Lista de objetos em campo `Vec<T>` é normalizada para um caminho por campo-chave. Objeto em campo `Map<K,V>` usa a chave como segmento.

### Fold policies

| Policy | Used for | Rule |
|--------|----------|------|
| `Lww` | escalar comum, `Raw`, `Option<T>` | entre candidatos válidos, vence o maior `Stamp`; empate com valores diferentes é `Error::Conflict` |
| `Flat` | campo anotado `#[colcha(flat)]`, discriminante de enum | todos os candidatos válidos MUST ser iguais; qualquer diferença é `Error::Conflict` |
| `Union` | `BTreeSet<T>`, `HashSet<T>` | união dos elementos; lista é achatada em elementos; ordem final é pela chave |
| `Atomic` | `Vec<T>` anotado `#[colcha(atomic)]` | a lista inteira é um valor; vence o maior `Stamp`; empate com listas diferentes é `Error::Conflict` |
| `Native` | campo nativo | escolhe um nome permitido; nomes diferentes são `Error::Conflict`; dobra os args contra `Native::args`, aplica `Native::coerce` e produz `Value::Native` |

`FoldPolicy` se aplica a folha. `Node` de struct e de mapa é estrutural e não usa `policy`.

Default de config é um fato `FactKind::Default`; default de schema é `Node.default`. A dobra usa candidato comum válido primeiro. Sem candidato comum válido, usa o default de config válido de maior `Stamp`. Sem default de config válido, usa `Node.default` se ele for válido. Entre defaults de config, a política do campo decide. Sem default válido e campo obrigatório, a dobra devolve `Error::Fold`. Campo `Option<T>` ausente é `None`, sem erro.

Default natural é `*default | type`. O derive preenche `Node.default` via `NodeOf::default_value()`:

| Tipo | `Node.default` |
|------|----------------|
| `bool` | `false` |
| `Option<T>` | `null` |
| `Vec<T>` | `[]` |
| `BTreeSet<T>`, `HashSet<T>` | `[]` |
| `BTreeMap<String, V>`, `HashMap<String, V>` | `{}` |
| enum com variante `#[default]` ou `#[colcha(default)]` | segmento da variante |
| struct cujos campos têm default | composição dos defaults |

`String`, inteiros, `f32`, `f64` e tipos nativos não têm default natural: o campo é obrigatório. Um campo sem default natural aceita ausência por `Option<T>`; não existe `Required<T>`. `#[colcha(default = <literal>)]` sobrescreve o default natural.

Tombstone é um fato `FactKind::Tombstone`. Ele absorve todo fato no mesmo caminho e em caminhos descendentes quando o `stamp` do fato é menor ou igual ao `stamp` do tombstone. Um fato posterior com `stamp` maior ressuscita o caminho.

Lista por chave dobra em `Value::List` ordenado pelo segmento da chave. A dobra preenche o campo-chave no map de cada elemento. A dobra de nativo valida os nomes dos argumentos contra `Native::args` e rejeita argumento desconhecido com `Error::Fold`.

### Derive attributes

| Attribute | Applies to | Rule |
|-----------|------------|------|
| `#[colcha(key)]` | campo `String` de elemento de `Vec<T>` | identifica o campo-chave; a dobra preenche o valor na materialização; escrita no caminho é `Error::Path` |
| `#[colcha(atomic)]` | campo `Vec<T>` | usa `FoldPolicy::Atomic`; a ordem é a do valor |
| `#[colcha(native)]` | campo de tipo nativo | emite `NodeKind::Native` com `NativeType::NAME`; a materialização chama `build` |
| `#[colcha(flat)]` | campo | usa `FoldPolicy::Flat` |
| `#[colcha(default = <literal>)]` | campo | sobrescreve `Node.default` com o literal |
| `#[colcha(default)]` | variante de enum | marca a variante default quando `#[default]` não é usado |
| `#[colcha(rename = "...")]` | campo ou variante | troca o segmento do caminho |
| `#[colcha(version = N)]` | struct raiz | define `Schema::VERSION`; o default é 1 |

`Vec<T>` sem `#[colcha(atomic)]` MUST ter exatamente um campo `#[colcha(key)]`. `#[colcha(key)]` fora de `Vec<T>` é erro de compilação. Atributo desconhecido é erro de compilação.

### EXAMPLE: alerta de monitoramento

```text
alert: {
  name:      string & =~"^[a-z0-9-]+$"
  threshold: float & >=0 & <=100
  severity:  *"warn" | "page" | "info"
  notify:    _core.Webhook { url: "https://example.test/hook" }
  tags:      *[] | ["db", "prod"]
  muted:     *false | bool
}
```

Três contribuições independentes somam em qualquer ordem. `threshold` faz meet. `tags` faz união. `severity` usa o default da variante. `muted` usa o default natural `false`. `notify` dobra os args do `Webhook` sem abrir nada. O único efeito é `Webhook::build` no `resolve`.

## Quality

| Concern | Measure; why it cannot happen |
|---------|----------------------------------|
| Compatibility | Cada `Fact` carrega `SchemaId`. `resolve` recusa identidade diferente de `T::NAME` + `T::VERSION` com `Error::Schema`. Mudança de normalização e de saída para o mesmo `State` é breaking change sob SemVer. |
| Error model | Toda operação pública falível devolve `Result<T, Error>`. Nenhuma entrada não confiável causa panic. As variantes de `Error` são estáveis para `match`; `Display` é informativo. |

## Security

In scope: parser de texto não confiável; limites de tamanho e profundidade; validação de caminho; nativos restritos ao registro do host; `build` restrito ao `resolve`.

Why it cannot happen (if claiming none): a crate não abre arquivo, socket, processo. Todo efeito está em `NativeHost::build`.

Residual risk: `NativeHost::build` pode fazer qualquer coisa; o host responde por sandbox e validação do nativo. Valores de config são `String` comuns e podem conter segredos; a crate não redige nem cifra o `State`.

## Success

- [ ] `proptest` com 10.000 estados aleatórios: qualquer ordem de `union` produz o mesmo `fold`.
- [ ] O mesmo `State` e `Schema` produzem o mesmo `Value` serializado em duas execuções.
- [ ] Dois candidatos `Lww` válidos com stamps diferentes: o de maior stamp vence.
- [ ] Candidato que viola constraint nunca vence candidato válido.
- [ ] Todos os candidatos inválidos com default: a dobra devolve o default. Sem default: `Error::Fold`.
- [ ] `bool`, `Option<T>`, coleções e enum com `#[default]` preenchem campo ausente; `String` e números sem default devolvem `Error::Fold`.
- [ ] `resolve` não chama `build` quando a dobra falha.
- [ ] `parse` rejeita `servers.foo.name` quando `name` é campo-chave.
- [ ] `resolve` rejeita estado com `SchemaId` diferente de `T::NAME` + `T::VERSION`.
- [ ] `compact` não muda o `fold` em 10.000 estados gerados.
- [ ] `Vec<Server>` materializa na ordem das chaves.
- [ ] Campo desconhecido na DSL devolve `Error::Path`.
- [ ] O parser não entra em panic com 10.000 entradas arbitrárias do `proptest`.
- [ ] Dois `kind` nativos válidos diferentes devolvem `Error::Conflict`.

## Later work

1. CLI `colcha` para parse, fold e resolve de arquivos.
2. Entrada JSON, TOML e YAML via serde.
3. Sequências de verdade (RGA, LSEQ).
4. IDs gerados por hash de conteúdo para `Vec<T>` sem campo-chave.
5. Campo `order` para ordenar mapas por algo além da chave.
6. Tipos nativos declarados pela DSL.
7. Mini-linguagem de despacho na config.
8. `check` com relatório de avisos, separado do `fold`.
9. Migração entre `SchemaId` diferentes.
10. Compactação incremental e backend de persistência.
11. `kind` de enum como `Lww`.
12. Segmentos de caminho Unicode.
13. Canal de avisos para candidatos inválidos descartados.
14. `no_std` e WASM.
15. Trait público `Fold` para políticas customizadas.

## Assumptions

| ID | Fact | If false |
|----|------|----------|
| AS-01 | O host fornece um relógio lógico que cresce a cada escrita no nó. | Stamps iguais podem produzir `Error::Conflict` em vez de ordem. |
| AS-02 | O autor da config aceita que candidato inválido seja descartado em silêncio na dobra. | É necessário um canal de avisos antes da v1. |
| AS-03 | Todo `Vec<T>` tem campo-chave; o campo sem chave é anotado `#[colcha(atomic)]`. | O derive falha na compilação e o tipo precisa de anotação. |
| AS-04 | O host controla o registro de nativos e mantém `NativeRegistry::spec` puro e estável. | A dobra deixa de ser determinística para o mesmo estado. |

## Decision history

- ADR-0001: merge é união de fatos. Rejeitados: meet de CUE como merge; override no mesmo operador.
- ADR-0002: DSL própria sem módulos. Rejeitados: engine CUE; entrada JSON/TOML/YAML na v1.
- ADR-0003: `Vec<T>` é mapa por chave. Rejeitados: RGA/LSEQ; IDs gerados; ordem de inserção.
- ADR-0004: nativos só no host e despacho no Rust. Rejeitados: DSL declarando nativos; mini-linguagem de despacho.
- ADR-0005: estado inválido é fail-closed. Rejeitados: fallback silencioso; marcação sem bloqueio.
- ADR-0006: override por folha e compactação explícita. Rejeitados: override por struct; compactação automática.
- ADR-0007: segmentos de `Path` são strings case-sensitive. Rejeitados: normalização de caixa; `%`-encoding; chave de mapa como categoria opaca.
- ADR-0008: `Raw` cobre valores heterogêneos. Rejeitados: traits em `Value`; caso especial de mapa; merge por chave dentro de `Raw`.
