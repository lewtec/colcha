# ADR-0006: Override por folha e compactação explícita

Status: accepted
Date: 2026-11-16

## Context

O override pode ser por campo. Também pode ser por struct inteira. A compactação pode rodar na inserção. Também pode rodar sob demanda. O estado cresce sem limite se fatos dominados ficam para sempre.

## Decision

Cada candidato tem seu `Stamp`. A política `Lww` escolhe por folha, no caminho do campo. Não existe metadado de override por struct na v1. `State::compact` é explícito, determinístico e recebe o `Node`; ele remove fatos dominados sem mudar o resultado da dobra. O parser limita entrada a 1 MiB e aninhamento a 32.

## Consequences

O caminho de escrita não carrega um segundo conjunto de metadados. A compactação é uma decisão do host, testável em separado. O limite de entrada protege o parser de texto não confiável.

## Rejected

- O override foi por struct inteira. O custo de metadados sobe e a composição por campo piora.
- A compactação rodou em cada inserção. O custo por escrita sobe e o merge deixa de ser uma união simples.
- A compactação foi omitida. O estado cresce sem limite em execução longa.
- O parser aceitou entrada sem limite. Texto não confiável pode esgotar memória e pilha.
