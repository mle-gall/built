# Steel profiles · IS: generator report

Written by scripts/build_steel_profiles.py. Every size is drawn with Built's parametric
families; area and mass (7850 kg/m³) are computed from the drawing and compared with the
listed values (limit 1 %, 2 % for sloped flanges).

200 sizes in the pack, 78 left out.

| Family file | Sizes | Largest area deviation | Largest mass deviation |
|---|---|---|---|
| ismb.csv | 14 | 1.13 % | 1.57 % |
| isjb.csv | 4 | 0.35 % | 0.41 % |
| islb.csv | 21 | 1.29 % | 1.70 % |
| iswb.csv | 14 | 0.34 % | 0.47 % |
| issc.csv | 9 | 0.65 % | 0.36 % |
| ishb.csv | 17 | 0.32 % | 0.13 % |
| ismc.csv | 20 | 0.60 % | 5.13 % |
| isjc.csv | 5 | 0.31 % | 0.45 % |
| islc.csv | 15 | 0.34 % | 0.46 % |
| ismcp.csv | 20 | 0.74 % | 0.67 % |
| isa-equal.csv | 74 | 3.08 % | 5.61 % |
| isa-unequal.csv | 65 | 3.08 % | 5.61 % |

## Left out

| Size | Family file | Why |
|---|---|---|
| ISLB 250 | islb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISLB 275 | islb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISLB 300 | islb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISLB 325 | islb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISLB 350 | islb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISLB 500 | islb.csv | the i family cannot draw it (invalid_profile). the standard prints T = 4.1 mm (likely 14.1); kept out until the current edition is checked |
| ISLB 600 | islb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISWB 250 | iswb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISWB 300 | iswb.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISSC 250 | issc.csv | the i family cannot draw it (invalid_profile). at the 8 degree slope the flange tip is thinner than the toe radius needs; the family cannot draw that toe yet |
| ISMC 175 x 22.7 | ismc.csv | computed mass 21.54 vs listed 22.7 (-5.13 %, limit 2 %). the standard's mass (22.7 kg/m) and area (27.6 cm², 21.7 kg/m) disagree |
| ISA 20 x 20 x 3 | isa-equal.csv | computed area 1.144 vs listed 1.12 (+2.17 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 20 x 20 x 4 | isa-equal.csv | computed area 1.474 vs listed 1.45 (+1.68 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 25 x 25 x 3 | isa-equal.csv | computed area 1.453 vs listed 1.41 (+3.08 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 25 x 25 x 4 | isa-equal.csv | computed area 1.883 vs listed 1.84 (+2.36 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 25 x 25 x 5 | isa-equal.csv | computed area 2.293 vs listed 2.25 (+1.93 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 30 x 30 x 3 | isa-equal.csv | computed area 1.764 vs listed 1.73 (+1.95 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 30 x 30 x 4 | isa-equal.csv | computed area 2.294 vs listed 2.26 (+1.49 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 30 x 30 x 5 | isa-equal.csv | computed area 2.804 vs listed 2.77 (+1.21 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 35 x 35 x 3 | isa-equal.csv | computed area 2.064 vs listed 2.03 (+1.66 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 35 x 35 x 4 | isa-equal.csv | computed area 2.694 vs listed 2.66 (+1.27 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 35 x 35 x 5 | isa-equal.csv | computed area 3.304 vs listed 3.27 (+1.03 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 35 x 35 x 6 | isa-equal.csv | computed mass 3.057 vs listed 3 (+1.88 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 40 x 40 x 3 | isa-equal.csv | computed area 2.375 vs listed 2.34 (+1.49 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 40 x 40 x 4 | isa-equal.csv | computed area 3.105 vs listed 3.07 (+1.14 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 40 x 40 x 6 | isa-equal.csv | computed mass 3.536 vs listed 3.5 (+1.04 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 45 x 45 x 3 | isa-equal.csv | computed area 2.675 vs listed 2.64 (+1.32 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 45 x 45 x 4 | isa-equal.csv | computed area 3.505 vs listed 3.47 (+1.01 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 50 x 50 x 3 | isa-equal.csv | computed area 2.987 vs listed 2.95 (+1.26 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 50 x 50 x 4 | isa-equal.csv | computed mass 3.075 vs listed 3 (+2.50 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 55 x 55 x 4 | isa-equal.csv | computed area 4.331 vs listed 4.26 (+1.66 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 55 x 55 x 5 | isa-equal.csv | computed area 5.341 vs listed 5.27 (+1.34 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 55 x 55 x 6 | isa-equal.csv | computed area 6.331 vs listed 6.26 (+1.13 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 55 x 55 x 8 | isa-equal.csv | computed mass 6.477 vs listed 6.4 (+1.20 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 60 x 60 x 5 | isa-equal.csv | computed area 5.841 vs listed 5.75 (+1.58 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 60 x 60 x 6 | isa-equal.csv | computed area 6.931 vs listed 6.84 (+1.33 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 60 x 60 x 8 | isa-equal.csv | computed area 9.051 vs listed 8.96 (+1.01 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 60 x 60 x 10 | isa-equal.csv | computed mass 8.706 vs listed 8.6 (+1.23 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 65 x 65 x 4 | isa-equal.csv | computed area 5.131 vs listed 5.04 (+1.80 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 65 x 65 x 5 | isa-equal.csv | computed area 6.341 vs listed 6.25 (+1.45 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 65 x 65 x 6 | isa-equal.csv | computed area 7.531 vs listed 7.44 (+1.22 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 70 x 70 x 5 | isa-equal.csv | computed area 6.855 vs listed 6.77 (+1.26 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 70 x 70 x 6 | isa-equal.csv | computed area 8.145 vs listed 8.06 (+1.06 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 75 x 75 x 5 | isa-equal.csv | computed area 7.355 vs listed 7.27 (+1.17 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 75 x 75 x 8 | isa-equal.csv | computed mass 9 vs listed 8.9 (+1.13 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 90 x 90 x 6 | isa-equal.csv | computed mass 8.317 vs listed 8.2 (+1.43 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 90 x 90 x 8 | isa-equal.csv | computed mass 10.92 vs listed 10.8 (+1.14 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 30 x 20 x 3 | isa-unequal.csv | computed area 1.453 vs listed 1.41 (+3.08 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 30 x 20 x 4 | isa-unequal.csv | computed area 1.883 vs listed 1.84 (+2.36 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 30 x 20 x 5 | isa-unequal.csv | computed area 2.293 vs listed 2.25 (+1.93 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 40 x 25 x 3 | isa-unequal.csv | computed area 1.914 vs listed 1.88 (+1.79 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 40 x 25 x 4 | isa-unequal.csv | computed area 2.494 vs listed 2.46 (+1.37 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 40 x 25 x 5 | isa-unequal.csv | computed area 3.054 vs listed 3.02 (+1.11 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 45 x 30 x 3 | isa-unequal.csv | computed area 2.214 vs listed 2.18 (+1.54 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 45 x 30 x 4 | isa-unequal.csv | computed area 2.894 vs listed 2.86 (+1.18 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 50 x 30 x 3 | isa-unequal.csv | computed area 2.375 vs listed 2.34 (+1.49 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 50 x 30 x 4 | isa-unequal.csv | computed area 3.105 vs listed 3.07 (+1.14 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 50 x 30 x 6 | isa-unequal.csv | computed mass 3.536 vs listed 3.5 (+1.04 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 60 x 40 x 5 | isa-unequal.csv | computed area 4.827 vs listed 4.76 (+1.41 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 60 x 40 x 6 | isa-unequal.csv | computed area 5.717 vs listed 5.65 (+1.19 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 65 x 45 x 5 | isa-unequal.csv | computed area 5.327 vs listed 5.26 (+1.28 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 65 x 45 x 6 | isa-unequal.csv | computed area 6.317 vs listed 6.25 (+1.08 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 65 x 45 x 8 | isa-unequal.csv | computed mass 6.466 vs listed 6.4 (+1.04 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 70 x 45 x 5 | isa-unequal.csv | computed area 5.591 vs listed 5.52 (+1.28 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 70 x 45 x 6 | isa-unequal.csv | computed area 6.631 vs listed 6.56 (+1.08 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 70 x 45 x 8 | isa-unequal.csv | computed mass 6.791 vs listed 6.7 (+1.35 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 75 x 50 x 5 | isa-unequal.csv | computed area 6.091 vs listed 6.02 (+1.17 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 75 x 50 x 6 | isa-unequal.csv | computed mass 5.676 vs listed 5.6 (+1.36 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 75 x 50 x 10 | isa-unequal.csv | computed mass 9.099 vs listed 9 (+1.10 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 80 x 50 x 5 | isa-unequal.csv | computed area 6.355 vs listed 6.27 (+1.36 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 80 x 50 x 6 | isa-unequal.csv | computed area 7.545 vs listed 7.46 (+1.14 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 80 x 50 x 10 | isa-unequal.csv | computed mass 9.503 vs listed 9.4 (+1.09 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 90 x 60 x 6 | isa-unequal.csv | computed area 8.761 vs listed 8.65 (+1.28 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 90 x 60 x 8 | isa-unequal.csv | computed mass 9.012 vs listed 8.9 (+1.26 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 100 x 65 x 6 | isa-unequal.csv | computed area 9.677 vs listed 9.55 (+1.33 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 100 x 75 x 6 | isa-unequal.csv | computed area 10.3 vs listed 10.1 (+1.93 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 100 x 75 x 8 | isa-unequal.csv | computed mass 10.61 vs listed 10.5 (+1.04 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
| ISA 125 x 75 x 10 | isa-unequal.csv | computed mass 15.05 vs listed 14.9 (+1.02 %, limit 1 %). toes marked 'reasonably square' (no R2) are drawn square; the standard's areas imply a toe radius, so these sizes wait for the current edition |
