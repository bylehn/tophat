import pandas as pd

frame = pd.read_csv("results/processed/summary.csv")
print(frame.groupby("group")["value"].mean())
