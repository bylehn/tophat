---
study: median-vs-mean
title: "Median vs mean filter on noisy step signals"
question: q-00000000
status: draft
visibility: public
variants:
  - base
  - impulse
---

# Median vs mean filter on noisy step signals

## Question

Does a median filter recover a noisy step signal better than a mean filter of the same width?

## Primary endpoint

The mean, over the signals of a variant, of RMSE(mean filter) − RMSE(median filter), each RMSE
taken against the clean signal. Positive values favour the median filter.

## Decision rule

The median filter is better if the 95% bootstrap interval of the endpoint (2000 resamples of the
signals, seed 1) lies entirely above 0. If the interval contains 0, the study shows no difference;
if it lies entirely below 0, the mean filter is better.

## Scenarios

```gherkin
Feature: median-vs-mean

  @S1
  Scenario: The endpoint is the mean RMSE difference
    Given the per-signal RMSEs of the mean and median filters
    When the analyse stage runs
    Then its result line reports the mean of RMSE(mean) − RMSE(median) and its 95% bootstrap interval

  @S2
  Scenario: Signals are reproducible
    Given the same parameters and seed
    When the generate stage runs twice
    Then it writes identical signal files

  @S3
  Scenario: The filter keeps every sample
    Given a generated signal
    When the filter stage runs
    Then its output has the clean, mean-filtered and median-filtered value of every input sample
```

## Variants

- `base`: Gaussian noise with standard deviation 0.3.
- `impulse`: impulse noise: each sample is replaced by −3 or +3 with probability 0.1.

## Stages

1. `generate` writes 30 seeded synthetic step signals of 200 samples, one file per signal. It is
   deterministic and checked by scenario S2, so it needs no QC verdict.
2. `filter` applies both filters (width 5) with a POSIX shell and awk script. It is an external
   stage: it runs on a cluster from a job folder that needs only POSIX tools.
3. `score` computes both RMSEs of each signal, one job per signal.
4. `analyse` computes the endpoint and its bootstrap interval and writes the result line.

## Amendments

## Result

## Interpretation
