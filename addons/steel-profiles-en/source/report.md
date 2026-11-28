# Steel profiles · EN: generator report

Written by scripts/build_steel_profiles.py. Every size is drawn with Built's parametric
families; area and mass (7850 kg/m³) are computed from the drawing and compared with the
listed values (limit 1 %, 2 % for sloped flanges).

1554 sizes in the pack, 12 left out.

| Family file | Sizes | Largest area deviation | Largest mass deviation |
|---|---|---|---|
| ipe.csv | 68 | 0.57 % | 3.15 % |
| he.csv | 124 | 0.50 % | 0.42 % |
| hl.csv | 39 | 1.09 % | 0.91 % |
| hd.csv | 42 | 0.38 % | 0.35 % |
| hp.csv | 31 | 0.49 % | 0.60 % |
| ubp.csv | 17 | 0.49 % | 0.09 % |
| ub.csv | 180 | 1.18 % | 1.45 % |
| uc.csv | 46 | 0.40 % | 0.70 % |
| ipn.csv | 21 | 0.36 % | 0.76 % |
| upe.csv | 14 | 0.30 % | 0.29 % |
| pfc.csv | 16 | 0.14 % | 0.29 % |
| upn.csv | 18 | 0.85 % | 0.84 % |
| l-equal.csv | 192 | 1.07 % | 2.22 % |
| l-unequal.csv | 32 | 0.34 % | 0.39 % |
| chs-hf.csv | 103 | 0.39 % | 0.44 % |
| shs-hf.csv | 123 | 0.37 % | 0.43 % |
| rhs-hf.csv | 161 | 0.42 % | 0.43 % |
| chs-cf.csv | 106 | 0.39 % | 0.44 % |
| shs-cf.csv | 96 | 0.47 % | 0.40 % |
| rhs-cf.csv | 137 | 0.47 % | 0.40 % |

## Kept with a nominal mass more than the limit away

These families list the designation's nominal mass, not a computed one; their area matches.

| Size | Family file | Computed mass | Nominal mass | Deviation |
|---|---|---|---|---|
| UB 610 x 325 x 217 | ub.csv | 219.5 | 217 | +1.15 % |
| UB 610 x 325 x 195 | ub.csv | 197.3 | 195 | +1.16 % |
| UB 610 x 325 x 174 | ub.csv | 175.9 | 174 | +1.07 % |
| UB 610 x 229 x 153 | ub.csv | 155.2 | 153 | +1.45 % |
| UB 533 x 165 x 74 | ub.csv | 74.79 | 74 | +1.07 % |

## Left out

| Size | Family file | Why |
|---|---|---|
| IPE V 450 | ipe.csv | computed mass 103.6 vs listed 107 (-3.15 %, limit 1 %). EN 10365:2017 itself lists G 107 kg/m with A 132.0 cm² (132 cm² is 103.6 kg/m): the standard's mass and area disagree |
| IPE AA 80 | ipe.csv | computed mass 4.95 vs listed 4.9 (+1.02 %, limit 1 %). the listed mass is rounded to 0.1 kg/m (4.9), within rounding of the computed 4.95 |
| HL 1100 A | hl.csv | computed area 440.8 vs listed 436 (+1.09 %, limit 1 %). the Orange Book's radius (r 35) gives more area than listed; check r1 against EN 10365 or the mill |
| UB 610 x 325 x 155 | ub.csv | computed area 199.3 vs listed 197 (+1.18 %, limit 1 %) |
| L 300 x 300 x 30 | l-equal.csv | computed mass 134.5 vs listed 133 (+1.13 %, limit 1 %) |
| L 300 x 300 x 26 | l-equal.csv | computed area 149.6 vs listed 148 (+1.07 %, limit 1 %) |
| L 110 x 110 x 14 | l-equal.csv | computed mass 22.78 vs listed 23.3 (-2.22 %, limit 1 %). listed mass and area disagree with each other in the Orange Book export (23.3 kg/m vs 29.0 cm²) |
| L 100 x 100 x 18 | l-equal.csv | computed mass 25.84 vs listed 26.2 (-1.38 %, limit 1 %). listed mass and area disagree with each other in the Orange Book export |
| L 90 x 90 x 5 | l-equal.csv | the angle family cannot draw it (invalid_profile). the listed toe radius r2 is larger than the leg thickness t, which no section can have |
| L 75 x 75 x 4 | l-equal.csv | the angle family cannot draw it (invalid_profile). the listed toe radius r2 is larger than the leg thickness t, which no section can have |
| L 65 x 65 x 4 | l-equal.csv | the angle family cannot draw it (invalid_profile). the listed toe radius r2 is larger than the leg thickness t, which no section can have |
| L 45 x 45 x 3 | l-equal.csv | the angle family cannot draw it (invalid_profile). the listed toe radius r2 is larger than the leg thickness t, which no section can have |
