# Steel profiles · AISC: generator report

Written by scripts/build_steel_profiles.py. Every size is drawn with Built's parametric
families; area and mass (7850 kg/m³) are computed from the drawing and compared with the
listed values (limit 1 %, 2 % for sloped flanges).

1537 sizes in the pack, 123 left out.

| Family file | Sizes | Largest area deviation | Largest mass deviation |
|---|---|---|---|
| w.csv | 289 | 0.73 % | 1.46 % |
| m.csv | 16 | 4.74 % | 11.38 % |
| s.csv | 28 | 1.33 % | 1.23 % |
| hp.csv | 22 | 2.25 % | 2.46 % |
| c.csv | 32 | 1.40 % | 6.95 % |
| mc.csv | 40 | 3.68 % | 5.08 % |
| l.csv | 137 | 3.02 % | 3.28 % |
| wt.csv | 289 | 0.73 % | 1.47 % |
| mt.csv | 14 | 4.85 % | 5.28 % |
| st.csv | 28 | 1.33 % | 1.23 % |
| hss-rect.csv | 399 | 0.38 % | 0.12 % |
| hss-square.csv | 126 | 0.34 % | 0.26 % |
| hss-round.csv | 189 | 0.70 % | 0.75 % |
| pipe.csv | 51 | 3.39 % | 0.35 % |

## Kept with a nominal mass more than the limit away

These families list the designation's nominal mass, not a computed one; their area matches.

| Size | Family file | Computed mass | Nominal mass | Deviation |
|---|---|---|---|---|
| W21X275 | w.csv | 414.1 | 409.245 | +1.17 % |
| W21X248 | w.csv | 373.9 | 369.065 | +1.32 % |
| W21X223 | w.csv | 336.7 | 331.861 | +1.46 % |
| W6X9 | w.csv | 13.56 | 13.393 | +1.26 % |
| M12.5X11.6 | m.csv | 17.5 | 17.263 | +1.37 % |
| M12X11.8 | m.csv | 17.85 | 17.56 | +1.64 % |
| M12X10.8 | m.csv | 16.46 | 16.072 | +2.39 % |
| M12X10 | m.csv | 15.16 | 14.882 | +1.84 % |
| M10X9 | m.csv | 13.79 | 13.393 | +2.97 % |
| M10X8 | m.csv | 12.44 | 11.905 | +4.45 % |
| M10X7.5 | m.csv | 11.37 | 11.161 | +1.86 % |
| M8X6.5 | m.csv | 10.18 | 9.673 | +5.29 % |
| M8X6.2 | m.csv | 9.38 | 9.227 | +1.66 % |
| M6X4.4 | m.csv | 6.635 | 6.548 | +1.32 % |
| M6X3.7 | m.csv | 5.571 | 5.506 | +1.17 % |
| M5X18.9 | m.csv | 28.42 | 28.126 | +1.05 % |
| M4X4.08 | m.csv | 6.677 | 6.072 | +9.97 % |
| M3X2.9 | m.csv | 4.807 | 4.316 | +11.38 % |
| HP10X57 | hp.csv | 85.77 | 84.825 | +1.11 % |
| HP10X42 | hp.csv | 63.78 | 62.503 | +2.04 % |
| HP8X36 | hp.csv | 54.89 | 53.574 | +2.46 % |
| C4X4.5 | c.csv | 6.881 | 6.697 | +2.75 % |
| C3X3.5 | c.csv | 5.571 | 5.209 | +6.95 % |
| MC18X58 | mc.csv | 87.41 | 86.314 | +1.27 % |
| MC18X51.9 | mc.csv | 78.3 | 77.236 | +1.37 % |
| MC18X45.8 | mc.csv | 69.18 | 68.158 | +1.50 % |
| MC18X42.7 | mc.csv | 64.62 | 63.545 | +1.70 % |
| MC13X50 | mc.csv | 75.7 | 74.408 | +1.73 % |
| MC13X40 | mc.csv | 60.8 | 59.527 | +2.13 % |
| MC13X35 | mc.csv | 53.31 | 52.086 | +2.36 % |
| MC13X31.8 | mc.csv | 48.58 | 47.324 | +2.66 % |
| MC12X31 | mc.csv | 46.69 | 46.133 | +1.21 % |
| MC12X14.3 | mc.csv | 21.54 | 21.281 | +1.20 % |
| MC12X10.6 | mc.csv | 16.07 | 15.775 | +1.87 % |
| MC10X41.1 | mc.csv | 62.01 | 61.164 | +1.39 % |
| MC10X33.6 | mc.csv | 50.83 | 50.002 | +1.65 % |
| MC10X28.5 | mc.csv | 43.23 | 42.413 | +1.92 % |
| MC10X25 | mc.csv | 38.07 | 37.204 | +2.32 % |
| MC10X22 | mc.csv | 33.51 | 32.74 | +2.35 % |
| MC10X8.4 | mc.csv | 12.86 | 12.501 | +2.89 % |
| MC10X6.5 | mc.csv | 10.06 | 9.673 | +4.05 % |
| MC9X25.4 | mc.csv | 38.57 | 37.799 | +2.03 % |
| MC9X23.9 | mc.csv | 36.29 | 35.567 | +2.03 % |
| MC8X22.8 | mc.csv | 34.6 | 33.93 | +1.98 % |
| MC8X21.4 | mc.csv | 32.51 | 31.847 | +2.07 % |
| MC8X20 | mc.csv | 30.39 | 29.763 | +2.10 % |
| MC8X18.7 | mc.csv | 28.47 | 27.829 | +2.30 % |
| MC8X8.5 | mc.csv | 13.13 | 12.649 | +3.78 % |
| MC7X22.7 | mc.csv | 34.38 | 33.781 | +1.77 % |
| MC7X19.1 | mc.csv | 29.03 | 28.424 | +2.14 % |
| MC6X18 | mc.csv | 27.28 | 26.787 | +1.83 % |
| MC6X15.3 | mc.csv | 23.18 | 22.769 | +1.79 % |
| MC6X16.3 | mc.csv | 24.77 | 24.257 | +2.11 % |
| MC6X15.1 | mc.csv | 22.97 | 22.471 | +2.22 % |
| MC6X12 | mc.csv | 18.28 | 17.858 | +2.37 % |
| MC6X7 | mc.csv | 10.91 | 10.417 | +4.75 % |
| MC6X6.5 | mc.csv | 10.16 | 9.673 | +5.08 % |
| MC4X13.8 | mc.csv | 20.8 | 20.537 | +1.29 % |
| MC3X7.1 | mc.csv | 10.99 | 10.566 | +4.03 % |
| L12X12X1-3/8 | l.csv | 158.6 | 156.257 | +1.52 % |
| L12X12X1-1/8 | l.csv | 131.4 | 129.768 | +1.28 % |
| L12X12X1 | l.csv | 117 | 115.779 | +1.08 % |
| L10X10X1-1/8 | l.csv | 108.7 | 107.594 | +1.02 % |
| L10X10X3/4 | l.csv | 73.81 | 73.069 | +1.02 % |
| L8X8X1/2 | l.csv | 39.68 | 39.288 | +1.00 % |
| L8X4X7/16 | l.csv | 25.92 | 25.596 | +1.26 % |
| L7X4X7/16 | l.csv | 23.7 | 23.364 | +1.44 % |
| L7X4X3/8 | l.csv | 20.45 | 20.239 | +1.05 % |
| L6X6X7/16 | l.csv | 25.92 | 25.596 | +1.26 % |
| L6X6X5/16 | l.csv | 18.8 | 18.453 | +1.87 % |
| L6X4X3/8 | l.csv | 18.55 | 18.304 | +1.35 % |
| L6X4X5/16 | l.csv | 15.63 | 15.328 | +1.95 % |
| L6X3-1/2X1/2 | l.csv | 23.06 | 22.769 | +1.29 % |
| L6X3-1/2X3/8 | l.csv | 17.6 | 17.412 | +1.09 % |
| L6X3-1/2X5/16 | l.csv | 14.83 | 14.584 | +1.72 % |
| L5X5X3/8 | l.csv | 18.55 | 18.304 | +1.35 % |
| L5X5X5/16 | l.csv | 15.63 | 15.328 | +1.95 % |
| L5X3-1/2X1/2 | l.csv | 20.47 | 20.239 | +1.12 % |
| L5X3-1/2X3/8 | l.csv | 15.64 | 15.477 | +1.05 % |
| L5X3-1/2X5/16 | l.csv | 13.19 | 12.947 | +1.84 % |
| L5X3-1/2X1/4 | l.csv | 10.65 | 10.417 | +2.28 % |
| L5X3X5/16 | l.csv | 12.39 | 12.203 | +1.55 % |
| L5X3X1/4 | l.csv | 10.02 | 9.822 | +2.03 % |
| L4X4X5/16 | l.csv | 12.34 | 12.203 | +1.11 % |
| L4X4X1/4 | l.csv | 9.965 | 9.822 | +1.46 % |
| L4X3-1/2X3/8 | l.csv | 13.68 | 13.542 | +1.05 % |
| L4X3-1/2X1/4 | l.csv | 9.332 | 9.227 | +1.14 % |
| L3-1/2X3X1/2 | l.csv | 15.35 | 15.179 | +1.10 % |
| L3-1/2X3X5/16 | l.csv | 9.96 | 9.822 | +1.41 % |
| L3-1/2X2-1/2X3/8 | l.csv | 10.84 | 10.715 | +1.13 % |
| L3-1/2X2-1/2X1/4 | l.csv | 7.433 | 7.292 | +1.94 % |
| L3X3X7/16 | l.csv | 12.49 | 12.352 | +1.12 % |
| L3X3X3/8 | l.csv | 10.84 | 10.715 | +1.13 % |
| L3X3X1/4 | l.csv | 7.433 | 7.292 | +1.94 % |
| L3X3X3/16 | l.csv | 5.687 | 5.521 | +3.00 % |
| L3X2-1/2X1/2 | l.csv | 12.81 | 12.649 | +1.31 % |
| L3X2-1/2X1/4 | l.csv | 6.8 | 6.697 | +1.54 % |
| L3X2-1/2X3/16 | l.csv | 5.211 | 5.045 | +3.28 % |
| L3X2X3/8 | l.csv | 8.89 | 8.78 | +1.26 % |
| L3X2X5/16 | l.csv | 7.535 | 7.441 | +1.27 % |
| L3X2X3/16 | l.csv | 4.687 | 4.569 | +2.59 % |
| L2-1/2X2-1/2X3/16 | l.csv | 4.649 | 4.569 | +1.76 % |
| L2-1/2X2X1/4 | l.csv | 5.449 | 5.387 | +1.15 % |
| L2-1/2X2X3/16 | l.csv | 4.173 | 4.092 | +1.99 % |
| L2-1/2X1-1/2X1/4 | l.csv | 4.816 | 4.747 | +1.45 % |
| L2-1/2X1-1/2X3/16 | l.csv | 3.697 | 3.631 | +1.83 % |
| L2X2X5/16 | l.csv | 5.912 | 5.834 | +1.34 % |
| L2X2X1/4 | l.csv | 4.816 | 4.747 | +1.45 % |
| L2X2X3/16 | l.csv | 3.697 | 3.631 | +1.83 % |
| L2X2X1/8 | l.csv | 2.521 | 2.455 | +2.69 % |
| WT10.5X137.5 | wt.csv | 207.3 | 204.623 | +1.33 % |
| WT10.5X124 | wt.csv | 187.2 | 184.532 | +1.47 % |
| WT10.5X111.5 | wt.csv | 168.3 | 165.93 | +1.46 % |
| WT6X7 | wt.csv | 10.53 | 10.417 | +1.07 % |
| WT3X4.5 | wt.csv | 6.781 | 6.697 | +1.25 % |
| MT6.25X5.8 | mt.csv | 8.75 | 8.631 | +1.38 % |
| MT6X5.9 | mt.csv | 8.924 | 8.78 | +1.64 % |
| MT6X5.4 | mt.csv | 8.22 | 8.036 | +2.29 % |
| MT6X5 | mt.csv | 7.57 | 7.441 | +1.74 % |
| MT5X4.5 | mt.csv | 6.895 | 6.697 | +2.96 % |
| MT5X4 | mt.csv | 6.221 | 5.953 | +4.50 % |
| MT5X3.75 | mt.csv | 5.687 | 5.581 | +1.91 % |
| MT4X3.25 | mt.csv | 5.092 | 4.837 | +5.28 % |
| MT4X3.1 | mt.csv | 4.69 | 4.613 | +1.67 % |
| MT3X2.2 | mt.csv | 3.317 | 3.274 | +1.32 % |
| MT3X1.85 | mt.csv | 2.785 | 2.753 | +1.17 % |
| MT2.5X9.45 | mt.csv | 14.21 | 14.063 | +1.05 % |

## Left out

| Size | Family file | Why |
|---|---|---|
| M12.5X12.4 | m.csv | computed area 23.7 vs listed 23.419 (+1.19 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M12.5X11.6 | m.csv | computed area 22.29 vs listed 21.935 (+1.63 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M12X11.8 | m.csv | computed area 22.74 vs listed 22.387 (+1.56 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M12X10.8 | m.csv | computed area 20.96 vs listed 20.516 (+2.18 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M12X10 | m.csv | computed area 19.31 vs listed 19.032 (+1.44 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M10X9 | m.csv | computed area 17.57 vs listed 17.097 (+2.76 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M10X8 | m.csv | computed area 15.84 vs listed 15.29 (+3.60 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M10X7.5 | m.csv | computed area 14.48 vs listed 14.323 (+1.11 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M8X6.5 | m.csv | computed area 12.97 vs listed 12.387 (+4.74 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M8X6.2 | m.csv | computed area 11.95 vs listed 11.742 (+1.76 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M6X4.4 | m.csv | computed area 8.452 vs listed 8.323 (+1.55 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M4X6 | m.csv | computed area 11.4 vs listed 11.29 (+1.02 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M4X4.08 | m.csv | computed area 8.506 vs listed 8.194 (+3.81 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| M3X2.9 | m.csv | computed area 6.124 vs listed 5.897 (+3.84 %, limit 1 %). M shapes: kdes - tf overstates the fillet (some M shapes have sloped flanges); needs ASTM A6 dimensions |
| HP10X57 | hp.csv | computed area 109.3 vs listed 107.742 (+1.41 %, limit 1 %) |
| HP10X42 | hp.csv | computed area 81.25 vs listed 80 (+1.56 %, limit 1 %) |
| HP8X36 | hp.csv | computed area 69.92 vs listed 68.387 (+2.25 %, limit 1 %) |
| MC18X51.9 | mc.csv | computed area 99.74 vs listed 98.709 (+1.05 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC18X45.8 | mc.csv | computed area 88.13 vs listed 87.097 (+1.19 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC18X42.7 | mc.csv | computed area 82.32 vs listed 81.29 (+1.27 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC13X50 | mc.csv | computed area 96.43 vs listed 94.839 (+1.68 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC13X40 | mc.csv | computed area 77.45 vs listed 75.484 (+2.60 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC13X35 | mc.csv | computed area 67.91 vs listed 66.451 (+2.20 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC13X31.8 | mc.csv | computed area 61.89 vs listed 60.322 (+2.60 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC12X31 | mc.csv | computed area 59.48 vs listed 58.839 (+1.09 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC12X14.3 | mc.csv | computed area 27.44 vs listed 26.968 (+1.73 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC12X10.6 | mc.csv | computed area 20.47 vs listed 20 (+2.36 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X41.1 | mc.csv | computed area 79 vs listed 78.064 (+1.19 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X33.6 | mc.csv | computed area 64.75 vs listed 63.677 (+1.68 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X28.5 | mc.csv | computed area 55.07 vs listed 54 (+1.98 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X25 | mc.csv | computed area 48.49 vs listed 47.355 (+2.40 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X22 | mc.csv | computed area 42.69 vs listed 41.613 (+2.58 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X8.4 | mc.csv | computed area 16.38 vs listed 15.871 (+3.24 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC10X6.5 | mc.csv | computed area 12.82 vs listed 12.581 (+1.91 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC9X25.4 | mc.csv | computed area 49.13 vs listed 48.193 (+1.95 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC9X23.9 | mc.csv | computed area 46.23 vs listed 45.29 (+2.07 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC8X22.8 | mc.csv | computed area 44.08 vs listed 43.226 (+1.98 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC8X21.4 | mc.csv | computed area 41.41 vs listed 40.516 (+2.21 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC8X20 | mc.csv | computed area 38.71 vs listed 37.871 (+2.22 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC8X18.7 | mc.csv | computed area 36.27 vs listed 35.484 (+2.20 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC8X8.5 | mc.csv | computed area 16.72 vs listed 16.129 (+3.68 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC7X22.7 | mc.csv | computed area 43.8 vs listed 43.032 (+1.77 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC7X19.1 | mc.csv | computed area 36.98 vs listed 36.193 (+2.18 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X18 | mc.csv | computed area 34.75 vs listed 34.129 (+1.81 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X15.3 | mc.csv | computed area 29.52 vs listed 28.968 (+1.92 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X16.3 | mc.csv | computed area 31.55 vs listed 30.903 (+2.10 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X15.1 | mc.csv | computed area 29.26 vs listed 28.645 (+2.15 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X12 | mc.csv | computed area 23.29 vs listed 22.774 (+2.26 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X7 | mc.csv | computed area 13.9 vs listed 13.484 (+3.08 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC6X6.5 | mc.csv | computed area 12.95 vs listed 12.581 (+2.91 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC4X13.8 | mc.csv | computed area 26.5 vs listed 26 (+1.92 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| MC3X7.1 | mc.csv | computed area 14 vs listed 13.613 (+2.86 %, limit 1 %). MC flange slopes vary by size and are not in the database; drawn parallel, so heavier |
| L7X4X7/16 | l.csv | computed area 30.19 vs listed 29.871 (+1.07 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X6X5/16 | l.csv | computed area 23.95 vs listed 23.677 (+1.14 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X4X9/16 | l.csv | computed area 34.62 vs listed 34.258 (+1.05 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X4X1/2 | l.csv | computed area 30.99 vs listed 30.645 (+1.13 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X4X7/16 | l.csv | computed area 27.37 vs listed 26.968 (+1.48 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X4X3/8 | l.csv | computed area 23.63 vs listed 23.29 (+1.47 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X4X5/16 | l.csv | computed area 19.91 vs listed 19.548 (+1.84 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X3-1/2X1/2 | l.csv | computed area 29.38 vs listed 29.032 (+1.19 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X3-1/2X3/8 | l.csv | computed area 22.42 vs listed 22.194 (+1.03 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L6X3-1/2X5/16 | l.csv | computed area 18.9 vs listed 18.645 (+1.35 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3-1/2X1/2 | l.csv | computed area 26.07 vs listed 25.806 (+1.03 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3-1/2X3/8 | l.csv | computed area 19.92 vs listed 19.677 (+1.25 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3-1/2X5/16 | l.csv | computed area 16.8 vs listed 16.516 (+1.70 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3-1/2X1/4 | l.csv | computed area 13.57 vs listed 13.355 (+1.63 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3X1/2 | l.csv | computed area 24.46 vs listed 24.194 (+1.09 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3X7/16 | l.csv | computed area 21.63 vs listed 21.355 (+1.30 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3X3/8 | l.csv | computed area 18.71 vs listed 18.452 (+1.41 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3X5/16 | l.csv | computed area 15.79 vs listed 15.548 (+1.54 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L5X3X1/4 | l.csv | computed area 12.77 vs listed 12.516 (+2.00 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X4X7/16 | l.csv | computed area 21.56 vs listed 21.29 (+1.28 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X4X3/8 | l.csv | computed area 18.64 vs listed 18.452 (+1.03 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X4X5/16 | l.csv | computed area 15.72 vs listed 15.484 (+1.51 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X4X1/4 | l.csv | computed area 12.69 vs listed 12.452 (+1.95 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X3-1/2X5/16 | l.csv | computed area 14.71 vs listed 14.516 (+1.32 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X3-1/2X1/4 | l.csv | computed area 11.89 vs listed 11.742 (+1.25 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X3X5/16 | l.csv | computed area 13.7 vs listed 13.484 (+1.59 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L4X3X1/4 | l.csv | computed area 11.08 vs listed 10.903 (+1.64 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3-1/2X3-1/2X5/16 | l.csv | computed area 13.7 vs listed 13.548 (+1.11 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3-1/2X3-1/2X1/4 | l.csv | computed area 11.08 vs listed 10.968 (+1.04 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3-1/2X2-1/2X5/16 | l.csv | computed area 11.68 vs listed 11.548 (+1.13 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3-1/2X2-1/2X1/4 | l.csv | computed area 9.469 vs listed 9.355 (+1.22 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X3X7/16 | l.csv | computed area 15.91 vs listed 15.677 (+1.49 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X3X3/8 | l.csv | computed area 13.8 vs listed 13.613 (+1.40 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X3X5/16 | l.csv | computed area 11.68 vs listed 11.484 (+1.70 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X3X1/4 | l.csv | computed area 9.469 vs listed 9.29 (+1.93 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X3X3/16 | l.csv | computed area 7.244 vs listed 7.032 (+3.02 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X2-1/2X1/2 | l.csv | computed area 16.32 vs listed 16.129 (+1.21 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X2-1/2X7/16 | l.csv | computed area 14.5 vs listed 14.323 (+1.23 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X2-1/2X3/8 | l.csv | computed area 12.59 vs listed 12.452 (+1.14 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X2-1/2X5/16 | l.csv | computed area 10.67 vs listed 10.516 (+1.46 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X2-1/2X1/4 | l.csv | computed area 8.663 vs listed 8.516 (+1.72 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L3X2-1/2X3/16 | l.csv | computed area 6.638 vs listed 6.452 (+2.88 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L2-1/2X2-1/2X3/8 | l.csv | computed area 11.28 vs listed 11.161 (+1.03 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L2-1/2X2-1/2X5/16 | l.csv | computed area 9.551 vs listed 9.419 (+1.40 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L2-1/2X2-1/2X3/16 | l.csv | computed area 5.923 vs listed 5.813 (+1.89 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L2X2X3/16 | l.csv | computed area 4.71 vs listed 4.658 (+1.11 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| L2X2X1/8 | l.csv | computed area 3.212 vs listed 3.168 (+1.37 %, limit 1 %). AISC gives no toe radius; without one the drawing is heavier than the listed area |
| MT6.25X6.2 | mt.csv | computed area 11.87 vs listed 11.742 (+1.08 %, limit 1 %). cut from M shapes, same fillet issue |
| MT6.25X5.8 | mt.csv | computed area 11.15 vs listed 10.968 (+1.62 %, limit 1 %). cut from M shapes, same fillet issue |
| MT6X5.9 | mt.csv | computed area 11.37 vs listed 11.226 (+1.27 %, limit 1 %). cut from M shapes, same fillet issue |
| MT6X5.4 | mt.csv | computed area 10.47 vs listed 10.258 (+2.08 %, limit 1 %). cut from M shapes, same fillet issue |
| MT6X5 | mt.csv | computed area 9.644 vs listed 9.548 (+1.00 %, limit 1 %). cut from M shapes, same fillet issue |
| MT5X4.5 | mt.csv | computed area 8.784 vs listed 8.581 (+2.37 %, limit 1 %). cut from M shapes, same fillet issue |
| MT5X4 | mt.csv | computed area 7.925 vs listed 7.677 (+3.23 %, limit 1 %). cut from M shapes, same fillet issue |
| MT5X3.75 | mt.csv | computed area 7.245 vs listed 7.161 (+1.17 %, limit 1 %). cut from M shapes, same fillet issue |
| MT4X3.25 | mt.csv | computed area 6.487 vs listed 6.187 (+4.85 %, limit 1 %). cut from M shapes, same fillet issue |
| MT4X3.1 | mt.csv | computed area 5.975 vs listed 5.877 (+1.66 %, limit 1 %). cut from M shapes, same fillet issue |
| MT3X2.2 | mt.csv | computed area 4.226 vs listed 4.174 (+1.24 %, limit 1 %). cut from M shapes, same fillet issue |
| MT2X3 | mt.csv | computed area 5.702 vs listed 5.645 (+1.02 %, limit 1 %). cut from M shapes, same fillet issue |
| HSS8X2X1/2 | hss-rect.csv | the rhs family cannot draw it (invalid_profile) |
| Pipe10STD | pipe.csv | computed area 71.74 vs listed 74.193 (-3.31 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe2STD | pipe.csv | computed area 6.469 vs listed 6.581 (-1.71 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe1STD | pipe.csv | computed area 2.994 vs listed 3.026 (-1.07 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe26XS | pipe.csv | computed area 240.7 vs listed 232.903 (+3.33 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe24XS | pipe.csv | computed area 221.8 vs listed 214.838 (+3.24 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe20XS | pipe.csv | computed area 184.1 vs listed 178.064 (+3.39 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe18XS | pipe.csv | computed area 165.3 vs listed 160 (+3.29 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe16XS | pipe.csv | computed area 146.4 vs listed 141.935 (+3.15 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe14XS | pipe.csv | computed area 127.6 vs listed 123.871 (+2.98 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe12XS | pipe.csv | computed area 115.8 vs listed 112.903 (+2.55 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
| Pipe12XXS | pipe.csv | computed area 222.8 vs listed 228.387 (-2.45 %, limit 1 %). the listed area uses a design wall the 0.93 rule does not reproduce for this size |
