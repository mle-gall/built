# Built Link for Blender

A live link between a Built model and Blender.

- **Built → Blender:** the model, one object per top-level placement in the
  "Built" collection, kept up to date as you edit in Built. Built's scene
  lights and scenes (as cameras) come along.
- **Blender → Built:** changed materials (base colour, roughness, metallic,
  image), lights, cameras and placement transforms. Built lists them in its
  Render drawer for Apply / Ignore; when both apps changed the same thing,
  you choose Keep Built or Take Blender. Mesh edits stay in Blender.

Install: in Built, Render drawer › Install Blender add-on… writes the zip.
In Blender, Edit › Preferences › Add-ons › Install from Disk…, pick it and
enable "Built Link". Then Render drawer › Open in Blender, or copy the
pairing code and use Built Link › Link to Built in the 3D Viewport sidebar.

The link only listens on this computer (127.0.0.1) and needs the pairing
code Built shows.
