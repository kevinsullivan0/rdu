# RDU final performance and practical fallback

**The delivered design keeps the simpler integrated0.3 scanner.** The ownership B experiment did not confirm a broad improvement over that control in the one final campaign, and used more peak RSS on all four inputs. The compact A+B combination was also not selected: its broad incremental speed effect was inconclusive, with a dense benefit and a wide-directory RSS regression. These consequential architectures were implemented and tested; this is a practical stopping decision, not an exhaustion or physical-optimum claim.

Public source `ea5969853f52770e627dfb60333194a182f9edc2` has exactly the same runtime/Cargo bytes as the integrated88 timed control, including the current UI, snapshots, journal and deletion behavior; the owner module is removed. The actual timed control was 1,340,928bytes, SHA-256 `3fb89b66e9112d20207b8e7a4b0ff9c8d38323d662f475f999527626fb23fe33`. Publication rebuilds that source with compiler paths remapped. **The privacy-remapped fallback release was not independently retimed.** Its final artifact digest and functional/security checks are supplied with the release receipt.

## Same-run comparison for the delivered design

All numbers below come from the same256 retained observations, with no new scanner/oracle child or resampling of input data. Each comparison uses all seven original paired rounds. The fallback was selected after seeing those results, so these finite unadjusted bootstrap intervals do not remove selection uncertainty. Speedup greater than one favors the integrated control.

| Input | Median wall ms | Median peak RSS MiB | vs Diskus default | vs Diskus8 | vs dua default | vs dua8 | vs GDU full16 |
|---|---:|---:|---:|---:|---:|---:|---:|
| cached-crates | 98.917 | 8.980 | 1.206× [0.908,1.461] | 1.218× [0.999,1.283] | 1.010× [0.700,1.099] | 0.951× [0.793,1.098] | 1.937× [1.824,2.086] |
| full-sdk | 251.416 | 17.906 | 0.834× [0.753,0.923] | 0.960× [0.900,1.027] | 0.779× [0.599,1.088] | 0.767× [0.710,0.885] | 1.650× [1.398,1.712] |
| skew | 112.407 | 14.230 | 1.602× [1.359,1.664] | 1.733× [1.519,1.889] | 1.080× [0.949,1.202] | 0.985× [0.917,1.171] | 3.279× [3.004,3.510] |
| uneven | 37.776 | 5.930 | 1.839× [1.742,1.976] | 1.971× [1.861,2.145] | 1.207× [1.106,1.510] | 1.239× [1.215,1.409] | 3.179× [3.126,3.374] |

The integrated control is1.65–3.28× faster than the included correct stock GDU full16 mode across these four inputs. Stronger scalar targets remain open: SDK favors default Diskus and fixed8dua; crates are unresolved against the stronger tools; skew and uneven favor RDU versus both Diskus policies, while uneven also favors RDU versus both dua policies. No universal superiority follows. GDU full16 is a correct bounded full-tree policy, not a claim it is GDU’s fastest policy on every input.

## What the one final campaign measured

Exactly256/256 declared process/accounting contracts passed:224 scalar and32 B-prototype retained-output observations; one warmup plus seven measured observations per case, fixed seed2026111057. All four independent before/after aggregate oracles/root identities, both real complete metadata models, zero-sentinel identity, five binary hashes and50 bound source/helper/receipt hashes stayed stable. Every warmup, negative direction and unresolved effect remains in performance.json. No retry, extra case or post-result optimization followed.

RDU scalar wall includes launch, full retained-tree construction/accounting, ordinary JSON output, destruction, worker joins and exit. CPU sums child workers and is not an elapsed-wall fraction. RSS is per-child maximum, not filesystem cache or all physical memory. Crates and the full SDK are exposed real inputs; skew was previously untimed by the selected experiment; uneven was sealed/created once after architecture freeze. Every input was warm from creation/oracles. One x86_64Mac/APFS laptop and seven finite unadjusted pairs constrain generalization.

Dua3.0.0 default and8 use the same isolated stock aggregate/no-sort/no-total/byte root+owned-zero-sentinel contract, with default omitting the thread flag. Actual worker counts were not instrumented. Diskus0.9.0 default/8 include directory metadata. GDU5.39.0 full16 uses the validated correct full-tree depth1 mode; its faster undercounting summary remains ineligible. Scalar tools retain less work but remain valid speed targets; error semantics differ.

## Rejected B prototype and retained-output evidence

The B/current paired speedups were crates0.943[0.836,1.316],SDK0.958[0.759,1.103],skew0.848[0.737,1.024],uneven0.990[0.798,1.012]. All point estimates are negative and all intervals cross one. B median RSS was9.914/18.191/14.914/6.262MiB versus current8.980/17.906/14.230/5.930. This supports the simpler practical fallback; it does not prove statistical equivalence or a physical optimum.

B’s genuine complete-retention/ascending-sort/full-output comparison versus stock dua8 remains an implemented prototype result: crates2.841[2.546,3.251],SDK1.875[1.610,2.463]. Every node/edge/subtotal/hardlink charge, size order and exposed count was checked in memory and real name text discarded. B RSS9.934/18.094MiB exceeded dua7.777/11.867. Formatting, equal-size ties, root+sentinel and destruction differ. **Those speedups do not establish the delivered fallback’s retained-output speed**, because that endpoint was not timed for it. Headless retained output also does not establish browser readiness. External dua/GDU controlled-PTY UI timing remains ineligible under recorded terminal capability limits.

## All resources, including unfavorable observations

Each row has seven measured observations. CPU is median user/system milliseconds; RSS is median/max peak MiB. B variants retain their original numeric data and are explicitly prototypes.

| Input | Variant | Median wall ms | SD ms | Min–max ms | User/system CPU ms | Median/max RSS MiB | Stdout bytes |
|---|---|---:|---:|---:|---:|---:|---:|
| cached-crates | integrated88 control | 98.917 | 23.314 | 81.675–156.697 | 66.724/419.074 | 8.980/9.023 | 119 |
| cached-crates | B prototype | 86.608 | 30.320 | 75.143–169.140 | 61.556/450.692 | 9.914/9.957 | 119 |
| cached-crates | diskus-default | 108.724 | 22.697 | 89.825–151.218 | 167.211/946.181 | 10.484/11.242 | 10 |
| cached-crates | diskus8 | 111.765 | 16.142 | 85.425–131.591 | 184.927/685.321 | 3.566/3.734 | 10 |
| cached-crates | dua-default | 91.065 | 22.010 | 68.324–133.045 | 114.919/451.770 | 4.383/4.438 | 194 |
| cached-crates | dua8 | 95.618 | 15.421 | 71.508–118.747 | 124.632/522.677 | 4.309/4.398 | 194 |
| cached-crates | gdu-full16 | 179.722 | 14.681 | 164.358–210.369 | 464.686/1077.416 | 31.715/33.645 | 57289 |
| full-sdk | integrated88 control | 251.416 | 35.407 | 222.705–336.025 | 158.761/1110.502 | 17.906/18.551 | 119 |
| full-sdk | B prototype | 265.386 | 36.445 | 232.442–336.924 | 160.077/1608.212 | 18.191/18.422 | 119 |
| full-sdk | diskus-default | 209.794 | 19.386 | 189.976–253.808 | 302.585/2257.334 | 12.828/13.629 | 10 |
| full-sdk | diskus8 | 241.693 | 81.484 | 213.979–455.617 | 348.179/1610.898 | 6.664/6.711 | 10 |
| full-sdk | dua-default | 180.250 | 42.954 | 165.080–274.375 | 256.403/1092.905 | 5.227/5.438 | 179 |
| full-sdk | dua8 | 190.899 | 55.466 | 159.678–297.520 | 270.488/1119.492 | 5.195/5.500 | 179 |
| full-sdk | gdu-full16 | 416.406 | 28.338 | 367.396–449.662 | 943.637/2646.159 | 51.215/52.152 | 461 |
| skew | integrated88 control | 112.407 | 17.403 | 100.003–153.581 | 48.889/373.753 | 14.230/14.289 | 109 |
| skew | B prototype | 129.700 | 18.594 | 111.460–166.365 | 47.369/418.354 | 14.914/15.391 | 109 |
| skew | diskus-default | 167.346 | 25.287 | 159.032–224.253 | 229.580/1577.870 | 21.691/22.668 | 2 |
| skew | diskus8 | 206.973 | 30.827 | 171.149–274.655 | 271.763/1194.219 | 16.391/16.562 | 2 |
| skew | dua-default | 121.330 | 17.353 | 106.818–159.580 | 147.981/503.402 | 4.305/4.508 | 210 |
| skew | dua8 | 121.262 | 13.939 | 101.748–140.847 | 147.921/480.273 | 4.270/4.672 | 210 |
| skew | gdu-full16 | 369.864 | 42.416 | 327.883–449.283 | 476.401/1195.348 | 47.770/50.312 | 432505 |
| uneven | integrated88 control | 37.776 | 3.845 | 32.689–44.638 | 21.950/150.990 | 5.930/6.062 | 107 |
| uneven | B prototype | 40.022 | 4.792 | 35.789–51.649 | 22.650/153.887 | 6.262/6.406 | 107 |
| uneven | diskus-default | 71.572 | 6.563 | 58.655–80.155 | 90.695/624.352 | 13.445/14.711 | 2 |
| uneven | diskus8 | 76.219 | 6.450 | 64.815–87.993 | 111.801/450.839 | 6.605/7.148 | 2 |
| uneven | dua-default | 46.173 | 3.966 | 42.974–54.691 | 57.267/168.199 | 3.895/3.965 | 209 |
| uneven | dua8 | 47.355 | 3.961 | 40.934–52.916 | 55.190/177.808 | 3.938/3.957 | 209 |
| uneven | gdu-full16 | 118.843 | 5.524 | 113.626–131.249 | 226.872/629.445 | 29.840/30.852 | 6662 |
| cached-crates | B full output | 143.839 | 34.929 | 127.388–239.438 | 112.823/490.130 | 9.934/10.082 | 1.63685e+06 |
| cached-crates | dua8 full output | 441.255 | 44.195 | 403.743–543.461 | 221.061/754.490 | 7.777/7.816 | 966498 |
| full-sdk | B full output | 396.170 | 94.662 | 352.608–601.519 | 268.488/1742.408 | 18.094/18.406 | 4.08701e+06 |
| full-sdk | dua8 full output | 833.768 | 135.532 | 708.386–1062.932 | 476.907/2300.702 | 11.867/11.926 | 2.14582e+06 |

[Complete sanitized samples and paired statistics](performance.json) preserve every numeric observation and exact command template. Actual terminal, deletion-safety, packaging and security gates accompany the release separately; this report invents no installed-browser latency or security approval. No filename inventory/raw process text/hardware identity is published. Private original raw SHA-256 `5e70bd8f7fbe262885d9149a336c976d7a979b3ca2098d770c3224ea9f758d57`.
