using Statistics

predictions = readlines("results/processed/predictions.csv")
println(mean(parse.(Float64, predictions[2:end])))
