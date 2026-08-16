# References and third-party notices

Sunk as a whole is distributed under GPL-3.0-only. The references below document research and visual inspiration; their source and assets are not redistributed in this repository.

The shader and Rust source in this repository are an independent implementation. No source or asset was copied from the following visual references.

## Mathematical and visual references

- rossning92/Blackhole: <https://github.com/rossning92/Blackhole>
  - Consulted for the Schwarzschild null-orbit formulation and visual comparison.
  - The inspected repository did not contain a license file, so its code and assets are not redistributed here.
- Reach036/BlackHole_Urp: <https://github.com/Reach036/BlackHole_Urp>
  - Consulted as an MIT-licensed Unity/URP visual reference for disk scale, color, and iteration-budget comparison.
  - Copyright (c) 2025 Reach. No file from that repository is included here.
- Zhihu article / Baopinsui: <https://zhuanlan.zhihu.com/p/20536269771>
- ShaderToy reference `4XcfR2` / EDLSDPSY: <https://www.shadertoy.com/view/4XcfR2>
  - Used only as visual and research pointers. Its content is not redistributed or translated line by line.
- NPGS / Baopinsui: <https://github.com/baopinshui/NPGS>
  - The current repository is GPL-3.0. No NPGS code is included here.
- Gargantua With HDR Bloom / sonicether: <https://www.shadertoy.com/view/lstSRS>
  - Acknowledged because the article identifies it as an upstream Bloom source. Its buffer code is not included here.

The inspected `BlackHole_Urp` main revision targets Unity `6000.2.10f1` with URP 17.2, not Unity 2022.3/URP 14. It remains useful as a visual reference, but exact Unity 2022.3 parity requires an older pinned revision or a separate baseline project. Its active shader also hard-codes zero spin; this project therefore describes the current model as Schwarzschild, not Kerr.

The physical identities used by this project—including `r_s = 2GM/c²`, the Schwarzschild photon-orbit equation, relativistic Doppler shift, and Beer–Lambert attenuation—are implemented from their mathematical definitions.
