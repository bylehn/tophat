---
study: x-vs-docking
title: "x-vs-docking"
question: q-deadbeef
status: draft
visibility: public
variants: [base]
---

# x-vs-docking

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

