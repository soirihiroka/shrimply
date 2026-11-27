Data, SQLite Queries, and Charts
================================

Reading data files
------------------

``read<T>(path)`` loads immutable snapshots during preparation and watches
files for hot reload:

.. code-block:: sanim

   type Sample = { id: String, value: Number }
   let samples = read<List<Sample>>("samples.csv")
   # Equivalent contextual inference:
   let other: List<Sample> = read("samples.csv")

* **CSV**: requires a record row schema and exact header; empty optional cells
  decode to ``None``.
* **JSON, TOML, YAML** (``.yaml`` / ``.yml``): validated recursively against
  the target schema. Duplicate YAML keys fail.
* **NPY / NPZ**: C-order finite numeric/boolean arrays; NPZ maps named arrays
  to record fields. Object/pickle arrays and Fortran order fail explicitly.
* **Plain text**: ``read<String>("notes.txt")`` reads verbatim UTF-8 (never
  interpolated).
* **Fonts**: ``Font("Google Sans, Arial, sans-serif")`` selects installed host
  families; ``read<Font>("font.ttf")`` loads an exact bundled font asset
  without silent glyph fallback.

Dictionaries and unstructured data
----------------------------------

``Dict`` holds string keys with mixed ``Data`` values. Use bracket access and
``decode<T>`` where a typed value is needed:

.. code-block:: sanim

   let settings = read<Dict>("settings.yaml")
   let title = decode<String>(settings["presentation"]["title"])
   let mixed: Dict = {"name": "demo", "count": 4, "nested": {"enabled": true}}
   let count = decode<Number>(mixed["count"])
   let fallback = decode<String>(unwrap(get(mixed, "subtitle"), Data("Untitled")))

``get`` returns ``Option<Data>``; ``keys`` lists dictionary keys.

SQLite queries
--------------

``query(...)`` runs against an in-memory SQLite database populated from bound
tables. Quoted SQL arguments use ``Template`` so ``{value}`` holes are safely
bound parameters (use ``SqlIdent(name)`` for quoted identifiers; concatenated
strings are rejected):

.. code-block:: sanim

   type Sample = { category: String, amount: Number }
   let samples = read<List<Sample>>("samples.csv")
   let minimum = 10
   let totals = query(
       "SELECT category, SUM(amount) AS total FROM samples WHERE amount >= {minimum} GROUP BY category ORDER BY category",
       tables={samples=samples},
   )
   for row in totals {
       let total = decode<Number>(row["total"])
   }

Use ``query<RowType>(...)`` to validate exact column names and types (nullable
columns require ``Option<T>``). Specify ``ORDER BY`` when row order matters.
For Pandas-style table pipelines built on the same SQLite engine, see
:doc:`dataframes`.

Charts and distributions
------------------------

Charts are ordinary elements sharing one explicit coordinate mapping
(``LinearScale``) and Adwaita default colors:

.. code-block:: sanim

   let xs = LinearScale(domain=[0.78, 1.28], range=[-275pt, 309pt])
   let ys = LinearScale(domain=[0, 4], range=[200pt, -200pt])
   let ticks = [0.8, 0.9, 1.0, 1.1, 1.2]
   let grid = <Grid scale={xs} ticks={ticks} spans={[[-200pt, 200pt]]} highlight={[1.0]}/>
   let axis = <Axis scale={xs} ticks={ticks} format="percent"/>
   type Observation = { x: Number, y: Number }
   let samples = read<List<Observation>>("samples.csv")
   let points = <Scatter data={samples} x_scale={xs} y_scale={ys} radius={2pt}/>
   let quartiles = {low=0.8, q1=0.9, middle=1.0, q3=1.1, high=1.2}
   let interval_box = <BoxPlot scale={xs} summary={quartiles}/>
   <Animation>
       {grid}
       {axis}
   </Animation>
   <Animation>
       {points.fade_in()}
       {interval_box.expand_x()}
   </Animation>

Available chart elements:

* **Scales, axes, grids, legends**: ``LinearScale(domain, range, clamp=...)``,
  ``scale_value(scale, value)``, ``DataRange(values, padding=0.05, include_zero=false)``,
  ``Axis`` (with ``format="number"``, ``"percent"``, or Python-style specs like
  ``.2f``, plus ``show_labels=false``), ``Grid``, ``Legend(labels=[...], colors=[...])``.
* **Scatter & box plots**: ``Scatter(data=List<{x:Number, y:Number}>, ...)``
  (optional ``keys`` and ``point_opacity``), ``BoxPlot``, ``BoxSummary(values)``.
* **Curves & bands**: ``LinePlot`` (``interpolation="linear"`` or
  ``"monotone"``), ``AreaPlot``, ``BandPlot`` (explicit ``low``/``high``),
  ``Polyline(points=..., width=2pt)``.
* **Distributions**: ``Histogram`` (``count``, ``probability``, or ``density``
  normalization; bin count or explicit ``edges``), ``DensityPlot`` (Gaussian
  KDE with Silverman automatic bandwidth or explicit ``bandwidth``),
  ``NormalCurve(mean, std_dev, ...)``, ``ECDFPlot``.

.. code-block:: sanim

   let xs = LinearScale(domain=[-4, 4], range=[-240pt, 240pt])
   let ys = LinearScale(domain=[0, 0.7], range=[100pt, -100pt])
   let measured = [-2, -1.5, -1, -0.4, 0, 0.2, 0.7, 1, 1.7, 2]
   let bars = <Histogram values={measured} bins={16} normalization="density" x_scale={xs} y_scale={ys}/>
   let estimate = <DensityPlot values={measured} bandwidth={0.5} x_scale={xs} y_scale={ys} fill="orange3"/>
   let model = <NormalCurve mean={0} std_dev={1} x_scale={xs} y_scale={ys} shade={false}/>
   [bars, estimate, model].create(duration=1s)
   wait(2s)

Materialize :doc:`DataFrames <dataframes>` at the plot boundary with
``df["value"].to_list()`` or ``df.select(["x", "y"]).to_list()``. See
`examples/adwaita_plots.sanim <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/adwaita_plots.sanim>`__.
