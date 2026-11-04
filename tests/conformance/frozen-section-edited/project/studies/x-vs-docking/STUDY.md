---
study: x-vs-docking
title: "x-vs-docking"
question: q-1a2b3c4d
status: preregistered
visibility: public
variants: [base]
preregistered:
  date: "2026-01-01"
  frozen_hash: "sha256:af9db24ee914948c0c4377091dbe1d83d05b3f8fbc1d1e627b4a548765f5b6fb"
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

