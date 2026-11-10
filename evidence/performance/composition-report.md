> Derived publication report for a rejected prototype screen. Numeric tables are preserved; internal next-step instructions and references to omitted private research artifacts are replaced by the final selection and evidence scope.

# PRODUCT-COMPOSITION-SCREEN-03: compact storage added to owner scheduling

This screen measures the complete retained product after adding compact representation A to worker-owned architecture B. The matched reference is standalone B. The five exposed inputs, seven measured pairs plus one warmup, seed and stopping rule were sealed before execution. This is development selection evidence; no A/B effect is assumed to add and no new strong-tool or usable-browser ratio follows.

## Frozen pair and exact endpoint

| Variant | Source | Binary bytes | SHA-256 |
|---|---|---:|---|
| B | `9830322c40c27bcb26600b3e9a91e4ff51e20dfe` | 1,349,288 | `3a1c920139e14d0403c2af335ab77b844355dd5a16546fbad5271895b0601cf5` |
| AB | `e622e58a746159f4a2420b6c565b68efbce547c8` | 1,349,440 | `4062972d337d6cff3f96eed8801b9402489b1f158e21c231239f8b7bf6fb97fc` |

**80/80 declared calls passed**, including ten warmups and seventy measured observations. Before/after independent metadata oracles, all bound source/build/helper hashes and both frozen executables remained stable. No additional scanner, competitor, profiler, browser or generator child belongs to this screen. The shared measurement lock was released after the recorded after-checks.

Each command was `FROZEN_BINARY --summary --read-only --backend bulk --threads 8 ROOT`. Primary wall spans launch, complete retained-tree construction and accounting, ordinary JSON output, tree destruction and process exit. Exact allocated/apparent totals, file and directory counts and zero errors matched the independent no-follow metadata oracle; stderr was empty. CPU sums child worker consumption and is not an elapsed fraction. RSS is per-process maximum, not allocator traffic or filesystem cache.

## Whole-command resources

Each row has seven measured observations. SD is population SD; CPU is median user/system milliseconds; RSS is median/largest per-child peak MiB.

| Input | Variant | Median wall ms | SD ms | Min–max ms | User/system CPU ms | Median/max peak RSS MiB | Stdout median bytes |
|---|---|---:|---:|---:|---:|---:|---:|
| cached-crates | B | 134.629 | 44.931 | 116.941–240.864 | 94.916/676.357 | 9.902/9.953 | 119 |
| cached-crates | AB | 140.910 | 27.386 | 110.159–190.103 | 90.020/712.102 | 8.379/8.477 | 119 |
| full-sdk | B | 361.135 | 68.797 | 302.977–484.062 | 231.343/2219.018 | 18.180/18.352 | 119 |
| full-sdk | AB | 372.677 | 44.069 | 307.350–438.803 | 216.968/2403.993 | 16.234/16.426 | 119 |
| flat-long | B | 109.759 | 15.179 | 100.260–147.725 | 17.823/87.185 | 8.906/9.793 | 104 |
| flat-long | AB | 111.496 | 19.754 | 94.722–156.478 | 15.478/88.755 | 10.738/10.832 | 105 |
| dense | B | 37.649 | 3.443 | 33.594–44.666 | 21.103/113.670 | 5.383/5.457 | 108 |
| dense | AB | 33.698 | 4.760 | 29.689–45.318 | 13.588/115.339 | 3.816/3.883 | 108 |
| chain | B | 38.405 | 12.649 | 34.200–73.039 | 7.342/26.316 | 1.398/1.730 | 102 |
| chain | AB | 37.654 | 2.758 | 35.489–43.692 | 7.283/25.803 | 1.430/1.516 | 102 |

## Paired wall effects

Speedup greater than one favors AB. Points are medians of paired log ratios; absolute savings are paired median differences. Intervals are 10,000 seeded paired percentile bootstrap resamples, unadjusted across five exposed comparisons. A crossing interval does not establish equivalence. Every negative and unresolved effect remains below.

| Input | AB speedup [95% CI] | Paired saved ms [95% CI] | AB faster /7 |
|---|---:|---:|---:|
| cached-crates | 1.046× [0.955,1.559] | 8.004 [-6.281,75.407] | 5 |
| full-sdk | 1.026× [0.880,1.202] | 11.449 [-43.856,77.369] | 4 |
| flat-long | 1.002× [0.984,1.058] | 0.230 [-1.737,5.538] | 4 |
| dense | 1.117× [1.039,1.177] | 3.952 [1.257,5.256] | 6 |
| chain | 0.985× [0.948,1.176] | -0.537 [-1.889,6.773] | 3 |

## Reported completed-tree duration

JSON elapsed time stops at the frozen source-defined retained scan/accounting boundary. Wall minus JSON duration combines launch/setup/output/drop/exit and does not isolate destruction. Per-sample medians need not subtract exactly.

| Input | Variant | Median JSON elapsed ms | JSON SD ms | Median wall−JSON gap ms |
|---|---|---:|---:|---:|
| cached-crates | B | 112.041 | 44.511 | 22.588 |
| cached-crates | AB | 122.125 | 26.760 | 18.229 |
| full-sdk | B | 328.502 | 67.355 | 27.733 |
| full-sdk | AB | 350.033 | 42.477 | 22.645 |
| flat-long | B | 92.154 | 12.857 | 17.605 |
| flat-long | AB | 93.901 | 17.080 | 15.884 |
| dense | B | 19.834 | 3.263 | 17.715 |
| dense | AB | 19.075 | 2.083 | 14.161 |
| chain | B | 24.324 | 10.271 | 14.081 |
| chain | AB | 24.154 | 2.393 | 13.339 |

## Bounded host observations

The future-only opt-in observer ran outside scanner timers under the same lock. Its declared commands were uptime, battery status and thermal status, bounded to two seconds each and bounded retained output. The complete observed/error/unknown records are in raw evidence. These reports do not measure CPU clocks, explain historical timing shifts or establish a thermal/physical floor.

| Stage | Observation | Status |
|---|---|---|
| before | uptime | observed |
| before | battery | observed |
| before | thermal_status | observed |
| after | uptime | observed |
| after | battery | observed |
| after | thermal_status | observed |

## Selection and scope

These warm exposed pairs left the broad compact-storage benefit inconclusive, with a dense-input benefit and a wide-directory RSS regression. The combination was rejected. The subsequent final comparison also did not confirm an owner-scheduler gain, so the delivered product uses the simpler original integrated scanner. This development screen is not a performance claim about that final executable.

[Curated numeric composition evidence](composition-evidence.json) preserves all numeric observations, resource summaries and paired effects. Command/config paths, host/process text and source inventories are omitted; the original raw SHA-256 and transformation are recorded in the evidence. No old experiment ratios are multiplied into final competitor claims.
