---
study: consumer
title: "consumer"
question: q-1a2b3c4d
status: reported
visibility: public
variants: [base]
preregistered:
  date: "2026-01-01"
  frozen_hash: "sha256:ad9992f769711774582dd24d5aad0e2ec8162f65f976bc140d120b735220efd4"
---

# consumer

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

