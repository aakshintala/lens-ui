# Spike A raw measurements (release, aarch64, real display)

Methodology: 560 frames, discard first 60; paint closure only (ms).
Windows opened on the real macOS display (not headless).

TSV columns: label, placement, paint_p50, paint_p95, paint_p99, snap_p95, input_to_first_ms, cache_hits, cache_misses

## S1

```
RESULT	full_redraw 80×24 S1	PerRow	1.509	2.939	2.967	0.044	2.946	0	0
RESULT	partial_update 80×24 S1	PerRow	0.619	1.953	3.358	0.010	2.932	0	0
RESULT	wide_and_sgr 80×24 S1	PerCell	1.744	3.725	3.753	0.001	5.671	0	0
RESULT	full_redraw 200×50 S1	PerRow	1.718	2.774	3.158	0.071	5.151	0	0
RESULT	partial_update 200×50 S1	PerRow	1.883	3.372	3.636	0.031	5.052	0	0
RESULT	wide_and_sgr 200×50 S1	PerCell	5.906	6.274	6.504	0.001	10.947	0	0
RESULT	full_redraw 400×100 S1	PerRow	5.360	5.964	6.044	0.108	10.511	0	0
RESULT	partial_update 400×100 S1	PerRow	5.787	6.440	6.639	0.006	10.500	0	0
RESULT	wide_and_sgr 400×100 S1	PerCell	16.130	16.486	16.725	0.001	23.152	0	0
```

## S2

```
RESULT	full_redraw 80×24 S2	PerRow	0.323	1.109	2.112	0.036	3.227	13416	24
RESULT	partial_update 80×24 S2	PerRow	0.451	1.659	2.205	0.010	2.896	11739	1701
RESULT	wide_and_sgr 80×24 S2	PerCell	1.648	3.165	3.720	0.001	6.350	0	0
RESULT	full_redraw 200×50 S2	PerRow	1.263	2.447	2.691	0.071	4.819	27950	50
RESULT	partial_update 200×50 S2	PerRow	1.479	2.662	3.153	0.032	5.127	26273	1727
RESULT	wide_and_sgr 200×50 S2	PerCell	6.009	6.387	6.506	0.001	11.500	0	0
RESULT	full_redraw 400×100 S2	PerRow	3.837	4.098	4.264	0.107	11.361	55900	100
RESULT	partial_update 400×100 S2	PerRow	4.124	4.499	4.666	0.005	10.956	54223	1777
RESULT	wide_and_sgr 400×100 S2	PerCell	16.224	16.671	16.898	0.001	24.058	0	0
```

Alignment: per-row `shape_line` probe (`a日b😀c`) MISALIGNED → wide_and_sgr used PerCell.
