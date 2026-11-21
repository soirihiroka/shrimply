Keyframed curves
================

``Curve(keys=[...])`` is an immutable, typed F-curve. Each ``Keyframe`` has an
exact timestamp and an ordinary numeric value. A scalar curve has type
``Curve<Number>`` (or ``Length``, ``Angle``, ``Percent``, ``Duration``, ``Rate``).
A vector curve uses a fixed numeric List, for example ``Curve<List<Length,2>>``.
Every key preserves the value's units and List length.

.. code-block:: text

   let size_keys = reflect([
       Keyframe(time=0s, value=24pt),
       Keyframe(time=1s, value=80pt),
       Keyframe(time=2s, value=40pt)
   ], label="Size curve")
   let size_curve = Curve(keys=size_keys)
   mut clock = 0s
   sig size = size_curve.sample(clock)
   let shape = <Box width={size} height={size} radius="full" fill="blue3" />
   Animation(children=[shape, animate(clock, to=2s, duration=2s, ease="linear")])

Reflect the key List to expose the timestamps, values, interpolation and handles
as editable inputs. Rebuilding a curve compiles its segments once; playback
shares those segments and performs a binary search for the requested time.
Seeking backward and forward does not integrate a simulation or evaluate Sanim
source code.

Sampling and animation
----------------------

``curve.sample(500ms)`` samples its exact clock. ``curve.at(0.5)`` samples normalized
progress through the same clock. Values before/after the endpoints hold the first
or final key; ``sample`` rejects negative or nonfinite time. ``curve.duration()``
returns the last timestamp as a ``Duration``.

A curve can drive a mutable value directly:

.. code-block:: text

   let movement = Curve(keys=[
       Keyframe(time=0s, value=0),
       Keyframe(time=1s, value=0.25),
       Keyframe(time=3s, value=1)
   ])
   mut progress = 0
   animate(progress, curve=movement)

``animate`` derives its duration from the curve. An explicit ``duration=6s``
retimes the same curve without altering its keys. Use exactly one of ``to=`` or
``curve=``; ``to=`` requires an explicit duration. Curves define their own timing,
so ``ease=`` is unavailable with ``curve=``. These plans compose and repeat using
the ordinary ``Animation`` layout. Camera position, target and zoom can consume
``curve.at(progress)`` or ``curve.sample(clock)`` signals just like other values.

Handles and interpolation
-------------------------

The outgoing key selects the segment interpolation:

* ``interpolation="bezier"`` uses cubic Bézier handles (the default).
* ``interpolation="linear"`` joins the two values at constant speed.
* ``interpolation="hold"`` keeps the outgoing value until the next timestamp.

``tangents="auto_clamped"`` is the default. It flattens extrema and limits automatic
handles to prevent overshooting adjacent values. ``"auto"`` uses smooth finite
difference tangents and permits overshoot. For manual handles, use ``"manual"``
and provide the handles consumed by each Bézier segment:

.. code-block:: text

   let travel = Curve(keys=[
       Keyframe(time=0s, value=0,
           tangents="manual", handle_out={time=300ms, value=0}),
       Keyframe(time=2s, value=1,
           tangents="manual", handle_in={time=-600ms, value=-0.1})
   ])

Handle ``time`` and ``value`` are offsets from their owning key. Incoming time is
nonpositive; outgoing time is nonnegative. A value offset uses the same units
and scalar/List shape as the key. Handles must remain ordered inside their
segment's time interval. Explicit handles override automatic handles on that
side of a key; missing automatic handles are derived from neighboring keys.

The first key starts at ``0s``. Timestamps are strictly increasing, without
duplicate-time keys. A curve has 2–4096 keys, at most 256 scalar channels and
65,536 key/channel components; its finite duration is at most two hours.
Serialization stores the editable keys and validates/recompiles them on load.

Geometric paths
---------------

``Curve(points=[[0,0],[10,20],[30,20]])`` describes a geometric path rather than
an F-curve. Its ``at(progress)`` samples distance along that path, and
``direction(progress)`` returns its spatial tangent. Geometric paths have no
clock: ``sample(time)``, ``duration()`` and ``animate(curve=...)`` require timestamped
keys. To control travel speed, animate progress with a keyframed scalar curve
and pass that progress to the geometric path.
