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
- compressed upper and lower images derived from that same disk;
- Doppler brightening and color asymmetry;
- approximate gravitational redshift;
- Kerr-like screen-space distortion and a lensed procedural star field;
- restrained bloom through the existing scene volume profile.

The upper disk structure is deliberately treated as a secondary lensed image, not a
second disk. Its height, thickness, span, and intensity are capped in
`GargantuaSettings` so it cannot dominate the primary disk.

## Tuning

Edit `Assets/Sunk/Settings/GargantuaSettings.asset`. The defaults are the reviewed
baseline for this milestone. Keep the following invariants while tuning:

```text
HorizonRadius < ApparentShadowRadius < DiskInnerRadius < DiskOuterRadius
SecondaryImageHeight <= 1.30 apparent shadow radii
SecondaryImageThickness <= 0.15 apparent shadow radii
SecondaryImageIntensity <= 0.65
```

The reviewed defaults are intentionally tighter: `1.08` height, `0.045` half
thickness, `1.72` horizontal half-span, and `0.28` intensity. The lower image is
further compressed and dimmed so the two lensed images do not read as a second
symmetrical disk.

The current lensing is a stable analytic approximation intended to establish the
composition and art direction. Numerical Kerr ray integration and measured quality
tiers remain later milestones.
