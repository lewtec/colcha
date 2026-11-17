# Validação do modelo da colcha contra ~/.dotfiles e modot

## Status

Experimental. O modelo cobre um subconjunto normalizado do config do modot.
Ele não representa o config real sem extensões. Referências, condicionais,
comprehensions e schema por módulo são resolvidos antes dos fatos (ADR-0012).

## Método

- Inspeção de `~/.dotfiles/modot.cue`, `~/.dotfiles/modules/*/modot.cue` e
  `~/.dotfiles/modot/internal/configcue/schema.cue`.
- Inspeção de `prelude_common.cue`, `prelude_home.cue`, `prelude_codebase.cue` e
  `internal/filespine/profile.go`.
- Config avaliado por `modot home config dump` e `modot home config eval`.
- Protótipo Rust em `/tmp/opencode/modot-validate` com o subconjunto tipado.
- Teste de compilação de `BTreeMap<String, Value>`.

## O que a colcha assume

- `State = BTreeSet<Fact>`. Cada fato tem `Path`, `Stamp`, `SchemaId` e `FactKind`.
- `Path` tem de 1 a 32 segmentos. Cada segmento é uma string não vazia e case-sensitive.
- O parser aceita segmento nu ou string entre aspas. A string pode conter `.`, `/`, `*` e maiúsculas.
- `Path::Display` cita segmentos que não são palavras nuas e junta os segmentos com `.`.
- O schema é estático e vem de `T::schema()`.
- Autoria com referências, condicionais, comprehensions e schema por módulo resolve antes de `State`. A colcha recebe fatos concretos.
- Structs derivadas são fechadas. Um campo `#[colcha(rest)]` aceita chaves desconhecidas.
- Enums derivados são unitários por padrão. `#[colcha(tag = "...")]` produz uma união tagueada com variantes nomeadas.
- `Vec<T>` com campo `#[colcha(key)]` é lista por chave. Sem chave, é atômica ou concat.
- `Raw` aceita qualquer `Value` concreto. Ele não tem caminhos filhos.
- `Value` não é um tipo de campo direto. Use `Raw`; ele implementa `NodeOf` e `Materialize`.

## O que mapeia diretamente

| Área | Modelo colcha | Evidência |
|---|---|---|
| `hosts` | `BTreeMap<String, Host>` | `riverwood`, `whiterun`, `ravenrock`, `phone` |
| `Host` | struct com `ips` atômico e opcionais | `#Host` |
| `lazy_tools` | `BTreeMap<String, LazyTool>` | `#LazyTool` |
| `modules` | `BTreeMap<String, ModuleRef>` | `#ModuleRef`; nomes são identificadores CUE |
| `inputs` | `BTreeMap<String, Input>` | `#Input` |
| `workspaces` | `BTreeMap<String, i64>` | `www: 1`, `meet: 2` |
| `desktop.wallpaper` | struct de strings | `dir`, `default` |
| `screenshot`, `browser`, `quicksync` | structs de escalares | prelude home |
| `runtime` | struct de escalares | `#Runtime` |
| `concurrency` | struct de inteiros | `#Concurrency` |
| `options.base16.mode` | enum `dark`/`light` | `*"dark" \| "light"` |
| `options.base16.palette.base00..base0f` | strings minúsculas | chaves minúsculas |
| `options.base16_<app>.palette` | struct aninhada | nomes de módulo são identificadores |
| `lint.tools.<name>.detect` | mapas aninhados | chaves `00-go-mod`, `10-rft` |
| `lint.tools.<name>.cmd` | `Vec<String>` atômico | `[...string]` |
| `file.home` / `file.codebase` / `file.system` | struct com 3 campos | `filespine.Profiles` |

O protótipo resolveu esse subconjunto. Exemplo: `hosts`, `lazy_tools`,
`modules` e `options.base16` com paleta minúscula.

## Lacunas

### P0: chaves arbitrárias (resolvido)

O config real usa chaves que não passavam na gramática antiga de `Path`.

Exemplos:

- `drivers."github.com/lewtec/modot/internal/driver/terminal.Driver"` tem `.`,
  `/` e maiúsculas.
- `file.home.".config/helix/config.toml"` tem `.`, `/` e começa com `.`.
- `options.base16.palette.base0A` tem maiúscula.
- `desktop.raw.dconf."org/gnome/desktop/interface"` tem `/`.
- `values."$schema"` começa com `$`.
- `values."markup.bold"` e `values."constant.character.escape"` têm `.`.
- `models."grok-4.6"` e `models."qwen3.5-9b"` têm `.`.

No dump CUE, 64 de 101 chaves citadas não são um segmento válido.
Treze têm maiúsculas. Sessenta e três têm ponto. Trinta e uma têm barra.
As treze maiúsculas são os caminhos Go dos drivers.
O dump CUE ainda omite `$schema`.

Nota sobre `base0A`: o nome canônico do slot amarelo na especificação Base16
usa letra hexadecimal maiúscula. Os nomes vão de `base00` a `base0F`.
O repositório não usa `base0a` em lugar nenhum.
O `base0a` do protótipo veio da normalização da colcha, não do modot.

O parser também normalizava chaves de mapa e de lista para minúsculo.
Isso impedia `base0A` e podia criar colisão entre `Base0A` e `base0a`.

Resolução: `Path` agora guarda strings case-sensitive. Cada segmento é uma
string não vazia. O parser aceita palavra nua ou literal entre aspas. `Display`
cita todo segmento que não seja palavra nua e usa `.` entre segmentos. A
codificação é reversível, então `a/b` (um segmento) e `a.b` (dois segmentos) não
colidem. Testes em `colcha/tests/core.rs` cobrem `base0A`, `base0a`, barras,
pontos e round-trip de `Display`.

### P0: valores arbitrários (resolvido)

`file.<profile>.<path>.values` e `options` carregam valores heterogêneos.

- `#FileStructured.values: [string]: _`.
- `options: [string]: _`.
- `desktop.raw.dconf: [string]: [string]: _`.
- `values` de `text` aceita `string | #SlotText | #SlotRef | #SlotLink`.
- `file.<profile>.<path>` é uma união de `#FileLines | #FileText | #FileRef |
  #FileLink | #FileStructured`.

A colcha não tinha um tipo de folha para JSON arbitrário.
`BTreeMap<String, Value>` não compilava, porque `Value` não implementa `NodeOf`
nem `Materialize`.

Resolução: `Raw` é um newtype sobre `Value`. Ele implementa `NodeOf` com
`NodeKind::Raw` e `Materialize`. O parser aceita objeto literal em campo `Raw`.
O fold usa LWW no valor inteiro. `Raw` não tem caminhos filhos e rejeita valor
nativo na materialização. `BTreeMap<String, Raw>` compila e cobre `values` e
`options`.

### P1: uniões com payload (resolvido)

O modot usa uniões etiquetadas:

- `#BackupActionGitRepoSync | #BackupActionRsync | #BackupActionArchive`.
- `#PlaceStepMove | #PlaceStepRequire`.
- `#Slot: string | #SlotText | #SlotRef | #SlotLink`.
- `#File: #FileLines | #FileText | #FileRef | #FileLink | #FileStructured`.

Resolução: `#[colcha(tag = "kind")]` transforma o enum em `NodeKind::Union` com
`tag` e `variants`. O fato é um mapa com o campo de etiqueta e os campos da
variante. O parser exige a etiqueta; o fold escolhe a variante, preenche defaults,
rejeita campo desconhecido e mantém a etiqueta no valor dobrado. `Option<T>`
aceita `null`. `Vec<T>` exige `#[colcha(atomic)]`.

`#Slot`, `#File` e as demais uniões que misturam string com struct continuam
representáveis por `Raw` na v1. Um enum tagueado não aceita variante escalar.

### P1: structs abertas (resolvido)

As structs derivadas são fechadas. O `lookup` rejeita campo desconhecido.

O modot usa abertura em vários lugares:

- `~/modot.cue` adiciona `a.b: "picuinha"` no topo.
- `#PlaceConfig` termina com `...`.
- `#gtkSettingsShared` termina com `...`.
- `options` é aberto por definição.

Resolução: `#[colcha(rest)]` em um campo `BTreeMap<String, V>` aceita chaves
desconhecidas. A chave vira caminho no objeto e a dobra usa o nó de `V`; a
materialização reúne as chaves no mapa. O segmento do campo rest não é gravável.
`#PlaceConfig` e `#gtkSettingsShared` cabem nesse mecanismo, e `options` segue
como mapa.

O topo de `~/modot.cue` continua fora de escopo: `a.b` adiciona um campo ao
schema, não um dado. A colcha tem schema estático.

### P1: semântica de listas (resolvido)

A colcha tem três modos: lista por chave, lista atômica e lista concat.

- Lista por chave exige um campo `String` e chaves válidas.
- Lista atômica usa LWW.
- Lista concat ordena os fatos por `(Stamp, Value)` e concatena os itens.
- `BTreeSet` usa união, mas ordena e deduplica.

O modot usa listas sem chave e com ordem:

- `backup.actions` é uma lista de uniões. A ordem importa. Cada camada pode
  contribuir itens.
- `lint.tools.<name>.cmd` é uma lista não vazia.
- `hosts.<name>.ips` é uma lista de strings.
- `bins` é uma lista de strings.

Resolução: `#[colcha(concat)]` em campo `Vec<T>`. A dobra ordena os candidatos
por `(stamp, valor)`, concatena os itens e preserva duplicatas. O merge continua
sendo união de fatos, então o resultado não depende da ordem de merge. Uma
contribuição inteira só sai por tombstone; não há retração por item na v1.
`min_len` continua disponível em `cmd`.

### P2: lista vazia (resolvido)

`Constraint` não tinha tamanho mínimo. O modot exige `[_, ...]` em `cmd` e em
outros lugares.

Resolução: `Constraint::MinLen(usize)` e `#[colcha(min_len = N)]` em campo
`Vec<T>`. A dobra avalia a lista final, então o default vazio e a lista por chave
também passam pela regra. Um fato `[]` no caminho de lista por chave é aceito e
representa a lista vazia.

### P2: ausente versus `null` (decisão: manter `Option<T>`)

A colcha colapsa os dois.

- `Option<T>` tem default `Value::Null`.
- `fold_leaf` devolve `Null` quando o campo opcional não tem fatos.
- `Materialize for Option<T>` transforma `Null` em `None`.

O CUE distingue campo ausente (`?`) de `null`. Em `mac?: string`, `null` não é
válido. Na colcha, `mac: null` vira `None` e passa.

Decisão da v1: manter o mapeamento de `Option<T>`. Ausência e `null` continuam
equivalentes; a diferença em relação ao CUE fica documentada aqui e no README.
Não há `Nullable<T>` na v1. Quem precisa rejeitar `null` usa `Raw` ou valida
antes de inserir o fato.

### P3: linguagem de avaliação (em escopo, na resolução)

O `modot.cue` usa recursos que não são fatos:

- referências: `palette: options.base16.palette`.
- condicionais: `if runtime.hostname == ...`.
- comprehensions: `for name, mod in modules ...`.
- interpolação: `"\(runtime.dotfiles_root)/..."`.
- defaults não literais.
- módulos com schema carregado em runtime.

Esses recursos estão no escopo, mas pertencem à fase de resolução. Ela lê a
autoria e emite `Fact` com `Value` concreto antes de `State`. `fold` e
`Materialize` nunca veem uma referência ou comprehension.

O fluxo é `autoria -> resolução -> Vec<Fact> -> State -> fold -> Materialize`.
Na v1 a resolução é o CUE do modot ou o host; um avaliador embutido entra antes
de `State` sem mudar a álgebra. Ver ADR-0012.

### P3: schema dinâmico por módulo (em escopo, na resolução)

Cada módulo declara `options: {...}`. O modot unifica esses schemas em runtime.

A colcha tem schema estático por tipo Rust. A resolução reduz o schema efetivo
ao schema do tipo, representa o resto como `Raw` ou campo rest e emite todos os
fatos com um `SchemaId` único. `T::schema()` e `fold::<T>` continuam estáticos;
`fold::<T>` rejeita schema estrangeiro.

Um avaliador embutido pode montar o schema antes de `State`, mas o contrato de
`SchemaId` não muda dentro da dobra.

## Prioridade

1. Chaves opacas com caixa preservada. Feito. Desbloqueia `file.home`, `drivers`,
   `base0A`, `dconf` e `$schema`.
2. Folha `Raw` com `NodeOf` e `Materialize`. Feito. Desbloqueia `values`, `options` e
   `desktop.raw`.
3. Enum com etiqueta externa. Feito. Desbloqueia `backup.actions` e `place.steps`.
4. Struct ou mapa aberto. Feito. Desbloqueia `#PlaceConfig`, `#gtkSettingsShared`
   e `options`.
5. Política de lista explícita. Feito com `#[colcha(concat)]`. Desbloqueia
   `actions`, `cmd` e `ips`.
6. `MinLen` e `Nullable`. `MinLen` feito com `#[colcha(min_len)]`. A v1 mantém
   `Option<T>` para ausência e `null`; `Nullable` fica fora.
7. Resolução de autoria (referências, condicionais, comprehensions,
   interpolação e schema por módulo). Em escopo, antes de `State`. O CUE do
   modot já faz isso; um avaliador embutido entra depois.

## Recomendação

Tratar a colcha como armazenamento de fatos normalizados, alimentado por um
avaliador. O avaliador pode ser o CUE do modot ou outro. A colcha valida o
subconjunto fechado e guarda o resto como `Raw`.

Essa leitura é compatível com o SPEC: sem módulos na DSL, com resolução antes do
`State` e com `fold` puro. Referências, comprehensions e schema dinâmico estão
no escopo, mas resolvem antes da materialização. Ver ADR-0012.

O avaliador embutido é trabalho posterior. Ele não muda `fold`, `State` nem
`Materialize`; só produz os fatos.

## Evidência

- `modot home config eval` gerou 1619 linhas e 40 KB.
- `modot home config dump` gerou 46 KB de JSON.
- O protótipo resolveu `hosts`, `lazy_tools`, `modules` e `options.base16` com
  paleta minúscula.
- O protótipo rejeitou `base0A`, `ui.background`,
  `.config/helix/config.toml` e o caminho Go do driver.
- `BTreeMap<String, Value>` falhou com `Value: NodeOf` e `Value: Materialize`
  não satisfeitos.
