# Sketch import fixtures

| File | Made from | How |
|---|---|---|
| `plate-mm.dxf` | drawn for these tests (lane 2) | text DXF, millimetres: an outline of lines and arcs, two slotted holes as polylines |
| `plate-mm.dwg` | `plate-mm.dxf` | the same LINE, ARC and LWPOLYLINE entities rewritten as a complete R2000 DXF (tables, block records) with ezdxf 1.4.5 (MIT), then written as DWG by GNU LibreDWG 0.14.8597 `dxf2dwg -y` (built from the release tarball `libredwg-0.14.8597.tar.xz`, sha256 `af2646681858a78d756cfb9e0eeb6901f61be3d09ae8f56a98e94b1490dec4ed`, the release the macOS build takes). LibreDWG's DXF reader drops the entities of a bare DXF without tables, hence the first step. Our own drawing: no third-party licence applies (the tools' licences cover the tools, not their output). `dwg2dxf` of the same release reads it back as 4 LINE, 2 ARC and 2 LWPOLYLINE entities |
| `notch-mm.svg` | drawn for these tests (lane 2) | a notch section in millimetres |
| `zed-inch-segmented.dxf` | drawn for these tests (lane 2) | a zed in inches with its bends as short segments (merge arcs, inch doubt) |
