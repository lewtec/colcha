# Validação do modelo da colcha contra ~/.dotfiles e modot

## Status

Experimental. O modelo cobre um subconjunto normalizado do config do modot.
Ele não representa o config real sem extensões.

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
- Structs derivadas são fechadas.
- Enums derivados têm apenas variantes unitárias.
- `Vec<T>` com campo `#[colcha(key)]` é lista por chave. Sem chave, é atômica.
- `Value` não é um tipo de campo. Ele não implementa `NodeOf` nem `Materialize`.

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

### P0: valores arbitrários

`file.<profile>.<path>.values` e `options` carregam valores heterogêneos.

- `#FileStructured.values: [string]: _`.
- `options: [string]: _`.
- `desktop.raw.dconf: [string]: [string]: _`.
- `values` de `text` aceita `string | #SlotText | #SlotRef | #SlotLink`.
- `file.<profile>.<path>` é uma união de `#FileLines | #FileText | #FileRef |
  #FileLink | #FileStructured`.

A colcha não tem um tipo de folha para JSON arbitrário.
`BTreeMap<String, Value>` não compila, porque `Value` não implementa `NodeOf`
nem `Materialize`.

Direção: criar um wrapper `Raw` ou `Json`. Ele implementa `NodeOf` e
`Materialize`. O nó de schema é uma folha sem constraint de tipo. O fold aceita
escalar, lista e mapa. O host valida o conteúdo depois, por `type`.

### P1: uniões com payload

O derive rejeita variantes com campos. O modot usa uniões etiquetadas:

- `#BackupActionGitRepoSync | #BackupActionRsync | #BackupActionArchive`.
- `#PlaceStepMove | #PlaceStepRequire`.
- `#Slot: string | #SlotText | #SlotRef | #SlotLink`.
- `#File: #FileLines | #FileText | #FileRef | #FileLink | #FileStructured`.

Direção: suportar enum com etiqueta externa. Exemplo: `#[colcha(tag = "kind")]`.
O fato é um mapa com o campo de etiqueta e os campos da variante. O fold valida a
etiqueta e exige os campos da variante. Para `#Slot` e `#File`, usar `Raw` na v1.

### P1: structs abertas

As structs derivadas são fechadas. O `lookup` rejeita campo desconhecido.

O modot usa abertura em vários lugares:

- `~/modot.cue` adiciona `a.b: "picuinha"` no topo.
- `#PlaceConfig` termina com `...`.
- `#gtkSettingsShared` termina com `...`.
- `options` é aberto por definição.

Direção: adicionar `#[colcha(open)]` ou um campo `extras`. O schema ganha um nó
de mapa para chaves não declaradas. A política de merge das extras precisa ser
explícita, por chave.

### P1: semântica de listas

A colcha tem dois modos: lista por chave e lista atômica.

- Lista por chave exige um campo `String` e chaves válidas.
- Lista atômica usa LWW.
- `BTreeSet` usa união, mas ordena e deduplica.

O modot usa listas sem chave e com ordem:

- `backup.actions` é uma lista de uniões. A ordem importa. Cada camada pode
  contribuir itens.
- `lint.tools.<name>.cmd` é uma lista não vazia.
- `hosts.<name>.ips` é uma lista de strings.
- `bins` é uma lista de strings.

União de `BTreeSet` não preserva ordem nem duplicatas. LWW perde contribuições de
outras camadas.

Direção: definir uma política de lista por campo. Opções: atômica, por chave,
concatenação, união ordenada. Não usar `BTreeSet` como padrão para listas de
valores.

### P2: lista vazia

`Constraint` não tem tamanho mínimo. O modot exige `[_, ...]` em `cmd` e em
outros lugares.

Direção: adicionar `Constraint::MinLen(usize)`. A origem pode ser
`#[colcha(min_len = 1)]` ou um wrapper `NonEmpty<T>`.

### P2: ausente versus `null`

A colcha colapsa os dois.

- `Option<T>` tem default `Value::Null`.
- `fold_leaf` devolve `Null` quando o campo opcional não tem fatos.
- `Materialize for Option<T>` transforma `Null` em `None`.

O CUE distingue campo ausente (`?`) de `null`. Em `mac?: string`, `null` não é
válido. Na colcha, `mac: null` vira `None` e passa.

Direção: manter ausência como ausência de fato. `Option<T>` não deve aceitar
`null` por padrão. Um wrapper `Nullable<T>` expressa `T | null`. A mudança mexe
no fold e no materialize.

### P3: linguagem de avaliação

O `modot.cue` usa recursos que não são fatos:

- referências: `palette: options.base16.palette`.
- condicionais: `if runtime.hostname == ...`.
- comprehensions: `for name, mod in modules ...`.
- interpolação: `"\(runtime.dotfiles_root)/..."`.
- defaults não literais.
- módulos com schema carregado em runtime.

A colcha não tem avaliação. O DSL é uma sintaxe de fatos com defaults e
constraints.

Isso não é lacuna se a colcha consumir config já avaliado. É bloqueante se a
colcha substituir a autoria CUE.

### P3: schema dinâmico por módulo

Cada módulo declara `options: {...}`. O modot unifica esses schemas em runtime.

A colcha tem schema estático. `T::schema()` é fixo em tempo de compilação.
`SchemaId` por fato e `fold::<T>` rejeita schema estrangeiro.

Direção: ou gerar um schema único para o workspace, ou montar o schema em
runtime a partir dos módulos. A segunda opção muda a assinatura de `resolve` e o
contrato de `SchemaId`.

## Prioridade

1. Chaves opacas com caixa preservada. Feito. Desbloqueia `file.home`, `drivers`,
   `base0A`, `dconf` e `$schema`.
2. Folha `Raw` com `NodeOf` e `Materialize`. Desbloqueia `values`, `options` e
   `desktop.raw`.
3. Enum com etiqueta externa. Desbloqueia `backup.actions` e `place.steps`.
4. Struct ou mapa aberto. Desbloqueia topo, `options` e `#PlaceConfig`.
5. Política de lista explícita. Desbloqueia `actions`, `cmd` e `ips`.
6. `MinLen` e `Nullable`. Fecham validações do CUE.

## Recomendação

Tratar a colcha como armazenamento de fatos normalizados, alimentado por um
avaliador. O avaliador pode ser o CUE do modot ou outro. A colcha valida o
subconjunto fechado e guarda o resto como `Raw`.

Essa leitura é compatível com o SPEC atual: sem módulos na DSL, sem avaliação e
com `fold` puro. Ela também evita reimplementar CUE.

Se a intenção for substituir a autoria CUE, o escopo cresce muito. Seria preciso
avaliação, referências, comprehensions, interpolação e schema dinâmico. Essa
decisão deve virar um ADR antes de qualquer implementação.

## Evidência

- `modot home config eval` gerou 1619 linhas e 40 KB.
- `modot home config dump` gerou 46 KB de JSON.
- O protótipo resolveu `hosts`, `lazy_tools`, `modules` e `options.base16` com
  paleta minúscula.
- O protótipo rejeitou `base0A`, `ui.background`,
  `.config/helix/config.toml` e o caminho Go do driver.
- `BTreeMap<String, Value>` falhou com `Value: NodeOf` e `Value: Materialize`
  não satisfeitos.
