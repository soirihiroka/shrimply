# DataFrames

`DataFrame(rows)` creates an immutable query plan over an existing `List<Record>`.
The rows remain in the same persistent, copy-on-write list storage. Chaining a
filter, projection, sort, aggregation, or join builds a bounded plan. SQLite
executes that plan when `to_list()`, a Series reduction, or `len()` requests values.
There is no Python/Pandas dependency and no second Rust row-processing engine.

```sanim
type Measurement = {case_name: String, before: Number, after: Number}
let df = DataFrame(read<List<Measurement>>("measurements.csv"))
let faster = df[df["after"] < df["before"]]
let ratios = faster.assign({ratio=faster["before"] / faster["after"]})
let summary = ratios.groupby("case_name").agg({before="mean", after="mean", ratio="mean"})
let result = summary.to_list()
let values = ratios["ratio"].to_list()
```

The familiar Pandas operations have a deliberately static contract:

| Operation | Result |
| --- | --- |
| `df["value"]` | A typed lazy `Series<ColumnType>` |
| `df[["x", "y"]]`, `df.select(["x", "y"])` | A projection with exactly those fields |
| `df[df["value"] > 0]` | A lazy row filter |
| `df.assign(ratio=df["x"] / df["y"])` | A frame with added/replaced columns |
| `df.drop(columns=["unused"])` | A projection excluding columns |
| `df.rename(columns={old_name="new_name"})` | A renamed schema |
| `df.sort_values("value", ascending=false)` | A stable sort; missing values last |
| `df.head(5)`, `df.tail(5)` | The first/last rows in current logical order |
| `df.groupby("category").agg({value="sum"})` | Grouped aggregation, group keys sorted, missing keys omitted |
| `df.groupby("category").mean()` | Mean of compatible numeric non-key columns |
| `df.merge(other, on="key", how="left")` | A typed inner/left join, with `_x`/`_y` collision suffixes |
| `df["value"].isna()`, `.fillna(0)` | A missingness mask / filled column |
| `df.dropna(subset=["value"])` | Rows without missing values in selected columns |
| `df.to_list()`, `df["value"].to_list()` | A typed materialized row/scalar list |
| `df["value"].mean()`, `.sum()`, `.count()`, `.min()`, `.max()` | A scalar SQLite reduction |
| `df.columns` | Column names in canonical schema order |
| `len(df)`, `len(df["value"])` | The number of rows, computed by SQLite |

Column names and schema-changing mappings must be literal strings, so unknown
columns, incompatible aggregates, duplicate output names, and invalid joins fail
before execution. A Series mask or assigned expression must come from the same
row plan. Build expressions from the filtered frame after filtering; unrelated
rows are never silently aligned. Use `merge` to combine frames explicitly.

Unlike mutable Pandas objects, methods return new plans and `let` bindings remain
immutable. Rows have a stable internal positional order, without a user-defined
Pandas index. Column ordering follows Sanim's canonical record schema ordering.
Both `groupby("one")` and `groupby(["one", "two"])` are supported. Aggregate
operations are `count`, `size`, `mean`, `sum`, `min`, and `max`; `agg` takes one
operation per selected column. Join modes are `inner` and `left`, with optional
`suffixes=["_left", "_right"]`. These supported operations are Pandas-inspired,
not an implementation of the entire Pandas library.

Missing data uses `Option<T>`. Empty numeric sums are zero; empty means and
minima/maxima are `None`. `count` ignores missing values; `size` counts rows.
Nullable arithmetic stays nullable, while comparisons produce total boolean
masks and treat missing operands as false. `isna()` tests missingness explicitly.
Division by zero or incompatible SQLite result values produce a source-located
error. Series arithmetic supports `+`, `-`, `*`, `/`, `%`, comparisons, and boolean
`and`/`or`/`not`. Other operators fail during static checking.

Materialized results share the existing query-result cache. Imported in-memory
SQLite snapshots also have a byte/entry-bounded LRU, so different queries over
unchanged input tables reuse their imports. Editing source data changes the
fingerprint; restoring a code checkpoint preserves its immutable plans. Cache
keys include exact limits and warm queries retain the original input cell and
instruction costs, so cache reuse cannot bypass a stricter budget. Plan depth,
node count, columns, SQL size, cells, pages, and SQLite VM work remain bounded.

Use `shrimply sanim lint` for static diagnostics and `shrimply sanim profile` to
measure actual query execution/cache reuse. Materializations inside loops receive
performance advice; slow executed queries report `P003`. Native charts consume
ordinary typed lists, for example `Histogram(values=df["value"].to_list())`.
The direct `query(...)` API remains available and uses the same SQLite executor.
