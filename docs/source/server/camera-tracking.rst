3D Camera Tracking
==================

Camera tracking reads a visual track and builds a camera path for a 3D scene.

Track a camera
--------------

#. Place the source footage on a visual track.
#. Select the 3D item that should use the tracked camera.
#. In the camera controls, set :guilabel:`Camera source` to the source visual
   track instead of :guilabel:`Custom`.
#. Choose a tracking method and analysis frame rate.
#. Select :guilabel:`Analyze` and keep the compute server running until the
   camera path is ready.

Available methods
-----------------

``COLMAP``
   Works on CUDA or CPU. Provides extra controls for quality and the camera
   model.

``VGGT-SLAM``
   Requires an **NVIDIA CUDA** device on the compute server. Hidden when the
   server is set to CPU.

Requirements and limits
-----------------------

* A lower analysis frame rate is faster and can miss quick camera moves. If the
  footage or the settings change, select :guilabel:`Analyze Again`.
* Very long clips are bounded by the server's 8 GiB frame archive limit.
