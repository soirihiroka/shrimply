Curves
======

Keyframed values
----------------

A ``Curve<T>`` maps time to a numeric scalar or fixed-size numeric List.

.. code-block:: text

   curve.sample(time: Duration): T
   curve.at(progress: Number): T
   curve.duration(): Duration

``sample`` uses an exact timestamp; ``at`` uses normalized progress 0..1.
Values outside the key interval hold the nearest endpoint. Negative times fail.

.. code-block:: sanim

   let movement = Curve(keys=[
       Keyframe(time=0s, value=0pt),
       Keyframe(time=1s, value=120pt)
   ])
   mut distance = 0pt
   animate(distance, curve=movement)

``animate`` takes either ``to=`` with a duration or ``curve=``. With a curve,
duration defaults to its last timestamp; an explicit duration retimes it.
``ease`` cannot be combined with ``curve``.

Keyframe arguments
------------------

``Keyframe(time: Duration, value: T, ...)`` returns a record used in
``Curve(keys=[...])``. All keys must have matching units and List lengths.

.. list-table::
   :header-rows: 1

   * - Argument
     - Type / default
     - Meaning
   * - ``time``
     - ``Duration``; required
     - Timestamp; first key is zero, subsequent times strictly increase.
   * - ``value``
     - Numeric scalar or fixed-size numeric List; required
     - Value at the key.
   * - ``interpolation``
     - ``"bezier"`` (default), ``"linear"``, ``"hold"``
     - How this key connects to the next.
   * - ``tangents``
     - ``"auto_clamped"`` (default), ``"auto"``, ``"manual"``
     - Automatic handles with/without overshoot limiting, or manual handles.
   * - ``handle_in`` / ``handle_out``
     - Optional ``{time: Duration, value: T}``
     - Offsets from this key; incoming time is nonpositive, outgoing nonnegative.

Reflect the key List to edit it in Shrimply. Curves allow 2–4096 keys, up to
256 channels and 65,536 key/channel components; finite duration is at most two
hours. Handles must stay ordered within each segment.

Geometric paths
---------------

``Curve(points=[[0,0], [10,20], [30,20]])`` defines a path without a clock.
``at(progress)`` samples distance along it; ``direction(progress)`` returns its
tangent. Animate a scalar progress value to control travel speed. Timed
``sample``, ``duration`` and ``animate(curve=...)`` require keyframes.
