---
study: external-study
title: "external-study"
question: q-1a2b3c4d
status: preregistered
visibility: public
variants: [base]
preregistered:
  date: "2026-01-01"
  frozen_hash: "sha256:ae7fc34b3e54038f3d053c86a1e2b9ea19562f99476d67d898eb1a041db8d8fb"
---

# external-study

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



## Interpretation

It works.

## Amendments

