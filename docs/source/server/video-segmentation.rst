Video Segmentation
==================

The :guilabel:`Segment Anything 2` modifier follows a subject through a video
and makes a mask.

Create a mask
-------------

#. Select a visual clip and add the :guilabel:`Segment Anything 2` modifier.
#. Move the playhead to a frame where the subject is clear.
#. Click the subject in the preview to add a foreground point. Right-click or
   Control-click to mark background that should be excluded.
#. Add more points as needed, or drag across the preview to draw a box around
   the subject.
#. Select :guilabel:`Analyze` and keep the compute server running until the
   mask is ready.

Adjust the result
-----------------

:guilabel:`Threshold` moves the mask edge. :guilabel:`Edge softness` feathers
it. :guilabel:`Invert` swaps inside and outside.

After you move a point, change the box, or pick another model, select
:guilabel:`Reanalyze`.

The server can offer SAM 2.1 tiny, small, base-plus, and large.
