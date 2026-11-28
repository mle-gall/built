# Steel profiles · AS/NZS: generator report

Written by scripts/build_steel_profiles.py. Every size is drawn with Built's parametric
families; area and mass (7850 kg/m³) are computed from the drawing and compared with the
listed values (limit 1 %, 2 % for sloped flanges).

121 sizes in the pack, 1 left out.

| Family file | Sizes | Largest area deviation | Largest mass deviation |
|---|---|---|---|
| ub.csv | 28 | 0.34 % | 1.19 % |
| uc.csv | 13 | 0.23 % | 1.02 % |
| ubp.csv | 4 | 0.38 % | 0.61 % |
| tfb.csv | 2 | 0.27 % | 0.34 % |
| pfc.csv | 10 | 0.17 % | 0.23 % |
| ea.csv | 46 | 0.35 % | 0.31 % |
| ua.csv | 19 | 0.30 % | 0.38 % |

## Kept with a nominal mass more than the limit away

These families list the designation's nominal mass, not a computed one; their area matches.

| Size | Family file | Computed mass | Nominal mass | Deviation |
|---|---|---|---|---|
| 310 UB 40.4 | ub.csv | 40.88 | 40.4 | +1.19 % |
| 150 UC 30.0 | uc.csv | 30.3 | 30 | +1.02 % |

## Left out

| Size | Family file | Why |
|---|---|---|
| 75 x 75 x 5 EA | ea.csv | the angle family cannot draw it (invalid_profile) |
