---
study: producer
title: "producer"
question: q-1a2b3c4d
status: reported
visibility: public
variants: [base]
preregistered:
  date: "2026-01-01"
  frozen_hash: "sha256:56880d85c7b1cd10e8367f26cb3b7437fd330c399f186ac8273f7864c51c57bb"
---

# producer

## Question

Does X work?

## Primary endpoint

A positive result.

## Decision rule

Positive means X works.

## Scenarios

```gherkin
Feature: x

  @S1
  Scenario: X works
    Given a variant "base"
    When the analyse stage completes
    Then the result is positive
```

## Variants

- base

## Stages

analyse

## Result

- base/analyse: n=2 level=b3f

## Interpretation

It works.

## Amendments

