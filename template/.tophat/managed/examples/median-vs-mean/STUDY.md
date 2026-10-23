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

- S1: The analyse stage's result line reports the mean of RMSE(mean) − RMSE(median) over the
  signals of a variant, and its 95% bootstrap interval.
- S2: The generate stage writes identical signal files when run twice with the same parameters
  and seed.
- S3: The filter stage's output has the clean, mean-filtered and median-filtered value of every
  input sample.

## Variants

- `base`: Gaussian noise with standard deviation 0.3.
- `impulse`: impulse noise: each sample is replaced by −3 or +3 with probability 0.1.

## Stages

1. `generate` writes 30 seeded synthetic step signals of 200 samples, one file per signal; its
   validation checks the number of signals, their length and their values. Trial runs of the
   draft explore on 10 signals with seed 2, so the 30 signals that decide stay unseen.
2. `filter` applies both filters (width 5) with a POSIX shell and awk script. It is an external
   stage: it runs on a cluster from a job folder that needs only POSIX tools. A human reviews its
   output: whether the median filter follows the step without the noise spikes.
3. `score` computes both RMSEs of each signal, one job per signal; its validation checks each
   signal's RMSEs.
4. `analyse` computes the endpoint and its bootstrap interval and writes the result line.

## Amendments

## Result

## Interpretation
