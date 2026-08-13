# Sunk Gargantua Prototype

This milestone establishes the first runnable black-hole renderer on the `unity` branch.
It is a full-screen mathematical effect built for Unity 6 and URP 17, rather than a
sphere, torus, particle system, or imported black-hole model.

## Open the prototype

Open `unity/Sunk` in Unity Hub, then load:

```text
Assets/Sunk/Scenes/GargantuaPrototype.unity
```

The product-owned `Assets/Sunk/Settings/Sunk_PC_Renderer.asset` contains the
`Sunk Gargantua` renderer feature. The effect is
visible in both Game view and Scene view and renders before the existing URP post
processing stack.

## Visual baseline

The prototype includes:

- an apparent black-hole shadow without sphere geometry;
- a narrow photon ring tied to the shadow boundary;
- one thin procedural accretion disk with a radial temperature gradient;
- continuous accretion-disk density integration with Beer-Lambert compositing;
- upper and lower higher-order images derived from real disk-plane crossings along the
  same integrated ray paths;
- Doppler brightening and color asymmetry;
- approximate gravitational redshift;
- Kerr-inspired finite-step ray bending, frame dragging, and a lensed procedural star field;
- multiple narrow photon-ring layers informed by ray turning and photon-sphere residency;
- restrained bloom through the existing scene volume profile.

The upper disk structure is deliberately treated as a secondary lensed image, not a
second disk. Its radiance is attenuated by disk-plane passage order, emission radius,
orbital winding, photon-orbit residence, and disk-face visibility so it cannot dominate
the primary disk. The legacy height/thickness/span controls remain serialized for project
compatibility; they no longer paint or clip a screen-space ellipse.

## Tuning

Edit `Assets/Sunk/Settings/GargantuaSettings.asset`. The defaults are the reviewed
baseline for this milestone. Keep the following invariants while tuning:

```text
HorizonRadius < DiskInnerRadius < DiskOuterRadius
HorizonRadius < ApparentShadowRadius < DiskOuterRadius
SecondaryImageHeight <= 1.30 apparent shadow radii
SecondaryImageThickness <= 0.15 apparent shadow radii
SecondaryImageIntensity <= 0.65
```

The reviewed transfer defaults use `0.115` for the first lensed image and `0.05` for
higher orders. The physical disk uses an inner radius of `2.15 Rs`, outer radius of
`9.10 Rs`, and half thickness of `0.048 Rs`. This permits the prograde inner disk to
extend inside the apparent shadow radius while remaining outside the configured horizon.

## Numerical model and limits

The current renderer traces 96 finite steps per pixel by default (configurable from 24
to 192), with adaptive step length and a capped direction change. It combines a Schwarzschild-inspired bending
term with a real-time frame-dragging approximation, records capture, escape, accumulated
turn, orbital winding, photon-sphere residency, and actual disk-plane crossings, and
samples the disk along every ray segment using a five-point composite Simpson density
estimate.

This model is intentionally described as **Kerr-inspired finite-step integration**. It is
not an exact integration of the Kerr metric in Boyer-Lindquist coordinates and is not
suitable for scientific measurement. Higher-order images are controlled only by integrated
trajectory and disk-transfer quantities; there is no screen-space arc mask or brightness
floor.

## Windows graphics backends

Windows Standalone is configured in this order:

1. Direct3D 12 (default)
2. Vulkan
3. Direct3D 11 (compatibility fallback)

The milestone is validated on both Direct3D 12 and native NVIDIA Vulkan. Their 1600x900
Player captures are visually equivalent (`49.72 dB` PSNR and `0.9992` global SSIM in the
current validation); the renderer does not depend on a backend-specific UV orientation or
screen flip.

Current Windows builds can log that URP's internal `DBufferClear` shader is unsupported.
URP strips the unused DBuffer variants while its global resource table still references
the shader. This project has no Decal renderer feature, and the Sunk pass writes only the
active color target, so the warning does not affect the current image on either backend.

## Reference boundary

The implementation was independently authored after reviewing the public concepts and
visual behavior in [Reach036/BlackHole_Urp](https://github.com/Reach036/BlackHole_Urp),
[rossning92/Blackhole](https://github.com/rossning92/Blackhole),
[the supplied Zhihu article](https://zhuanlan.zhihu.com/p/20536269771), and
[the supplied ShaderToy scene](https://www.shadertoy.com/view/4XcfR2). No source code or
assets from `rossning92/Blackhole`, which does not declare a repository license, are included.
