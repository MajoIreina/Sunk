#ifndef SUNK_GARGANTUA_DISK_INCLUDED
#define SUNK_GARGANTUA_DISK_INCLUDED

float3 SunkTemperatureColor(float temperatureKelvin)
{
    // Deliberately return linear HDR radiance. The ordering gives a red/copper
    // receding side while a blueshifted surface reaches cold white before blue.
    float emberBlend = smoothstep(1050.0, 2550.0, temperatureKelvin);
    float goldBlend = smoothstep(2350.0, 4050.0, temperatureKelvin);
    float whiteBlend = smoothstep(3900.0, 6250.0, temperatureKelvin);
    float coldBlend = smoothstep(5900.0, 9300.0, temperatureKelvin);
    float3 deepEmber = float3(0.68, 0.022, 0.0015);
    float3 copper = float3(1.18, 0.16, 0.016);
    float3 gold = float3(1.62, 0.56, 0.095);
    float3 warmWhite = float3(1.78, 1.42, 1.08);
    float3 coldWhite = float3(1.52, 1.72, 2.08);
    float3 color = lerp(deepEmber, copper, emberBlend);
    color = lerp(color, gold, goldBlend);
    color = lerp(color, warmWhite, whiteBlend);
    return lerp(color, coldWhite, coldBlend);
}

float3 SunkDiskEmission(
    float diskRadius,
    float orbitalToObserver,
    float dopplerFactor,
    float gravityShift,
    float2 flowCoordinate,
    float innerRadius,
    float outerRadius,
    float diskTemperature,
    float emission,
    float turbulence,
    float rotationSpeed,
    float dopplerStrength,
    float redshiftStrength)
{
    float radial01 = saturate((diskRadius - innerRadius) / max(outerRadius - innerRadius, 0.001));
    float inverseRadiusProfile = pow(
        max(innerRadius / max(diskRadius, innerRadius), 0.001),
        0.75);
    float zeroTorqueProfile = pow(
        saturate(1.0 - sqrt(innerRadius / max(diskRadius, innerRadius + 0.001))),
        0.25);
    float temperatureProfile = inverseRadiusProfile * lerp(0.58, 1.0, zeroTorqueProfile);

    // flowCoordinate.x is logarithmic radius and y is a four-unit azimuth.
    // Mapping the angle through sin/cos makes every noise octave periodic at
    // the cylindrical seam. Differential angular advection supplies Keplerian
    // shear without incorrectly translating the texture through the radius.
    float radiusRatio = max(diskRadius / max(innerRadius, 0.001), 1.0);
    float keplerRate = pow(rcp(radiusRatio), 1.5);
    float orbitalPhase = flowCoordinate.y * (0.5 * SUNK_PI) -
        _Time.y * rotationSpeed * (0.22 + 2.65 * keplerRate);
    float2 orbitalCircle = float2(cos(orbitalPhase), sin(orbitalPhase));
    float2 secondCircle = float2(cos(orbitalPhase * 2.0), sin(orbitalPhase * 2.0));

    float macroCloudA = SunkFbm(
        float2(
            flowCoordinate.x * 0.58 + orbitalCircle.x * 1.32,
            orbitalCircle.y * 1.18 - flowCoordinate.x * 0.16) +
        float2(4.7, -2.1));
    float macroCloudB = SunkFbm(
        float2(
            flowCoordinate.x * 1.07 + secondCircle.x * 0.72,
            secondCircle.y * 0.78 + flowCoordinate.x * 0.21) +
        float2(-7.4, 5.3));
    float macroCloud = macroCloudA * 0.72 + macroCloudB * 0.28;

    float cloudWarp = (macroCloud - 0.48) * 4.1;
    float finePhase = orbitalPhase * 3.0 + cloudWarp * 0.42;
    float2 fineCircle = float2(cos(finePhase), sin(finePhase));
    float fineCloud = SunkFbm(
        float2(
            flowCoordinate.x * 3.15 + fineCircle.x * 1.85,
            fineCircle.y * 2.15 - flowCoordinate.x * 0.44) +
        float2(11.6, -8.9));

    float cloudRaw = macroCloud * 0.78 + fineCloud * 0.22;
    float cloudMass = smoothstep(0.245, 0.725, cloudRaw);

    // Integer angular harmonics keep the cylindrical seam closed, while the
    // non-integer radial terms and cloud-warped cross phases stop the strands
    // from reading as uniformly spaced record grooves.
    float phaseScatter =
        sin(orbitalPhase * 3.0 + flowCoordinate.x * 1.37 + macroCloudB * 2.3) * 0.48 +
        sin(orbitalPhase * 7.0 - flowCoordinate.x * 0.83 + macroCloudA * 3.1) * 0.27;
    phaseScatter += (macroCloudA - macroCloudB) * 2.15 + (fineCloud - 0.48) * 1.42;
    float broadPhase = flowCoordinate.x * 5.47 - orbitalPhase * 2.0 +
        cloudWarp * 1.28 + phaseScatter * 0.74;
    float filamentPhase = flowCoordinate.x * 13.73 + orbitalPhase * 5.0 -
        cloudWarp * 1.12 + phaseScatter * 1.31;
    float dustPhase = flowCoordinate.x * 7.29 + orbitalPhase * 3.0 +
        cloudWarp * 0.81 - phaseScatter * 0.93;
    float broadFilament = pow(saturate(0.5 + 0.5 * sin(broadPhase)), 2.2);
    float fineFilament = pow(saturate(0.5 + 0.5 * sin(filamentPhase)), 5.0);
    float dustLane = pow(
        saturate(0.5 + 0.5 * sin(dustPhase + 2.1)),
        7.0);
    float structureStrength = saturate(turbulence);
    float cloudEmission = lerp(1.0, 0.16 + 1.62 * cloudMass, structureStrength);
    float selfExtinction = exp2(
        -structureStrength * (0.78 * (1.0 - cloudMass) + 1.72 * dustLane));
    float filamentEmission = 1.0 + structureStrength *
        (0.33 * broadFilament + 0.62 * fineFilament * lerp(0.34, 1.0, cloudMass));
    float filaments = cloudEmission * selfExtinction * filamentEmission;

    // I_nu / nu^3 is invariant along the ray, so the same combined frequency
    // shift drives both observed temperature and radiance. There is no upper
    // clamp here: tone mapping and bloom consume the resulting linear HDR peak.
    float dopplerShift = lerp(1.0, max(dopplerFactor, 0.001), saturate(dopplerStrength));
    float gravitationalShift = lerp(1.0, max(gravityShift, 0.001), saturate(redshiftStrength));
    float frequencyShift = max(dopplerShift * gravitationalShift, 0.001);
    float shiftedTemperature = diskTemperature * temperatureProfile * frequencyShift;
    float gCubed = frequencyShift * frequencyShift * frequencyShift;

    // Keep the body of the disk quieter, but retain a compact, unclamped HDR
    // core where the combined frequency shift, orbit and inner annulus agree.
    // At full alignment 0.85 * (1 + 1.35) is approximately twice the old peak.
    float innerCore = 1.0 - smoothstep(0.34, 0.62, radial01);
    float approachingCore = smoothstep(1.07, 1.23, frequencyShift) *
        smoothstep(0.28, 0.90, orbitalToObserver) * innerCore *
        lerp(0.72, 1.0, cloudMass);
    float frequencyFlux = gCubed * 0.85 * (1.0 + 1.35 * approachingCore);

    float approachingWing = smoothstep(1.02, 1.42, dopplerShift) *
        smoothstep(-0.05, 0.82, orbitalToObserver);
    float recedingWing = (1.0 - smoothstep(0.66, 0.98, dopplerShift)) *
        smoothstep(-0.05, 0.82, -orbitalToObserver);

    float edgeFade = smoothstep(0.0, 0.045, radial01) * (1.0 - smoothstep(0.80, 1.0, radial01));
    float middleValley = 0.72 + 0.28 * abs(radial01 * 2.0 - 1.0);
    float innerGlow = 1.0 + 1.25 * exp2(-radial01 * 9.0);
    float3 thermalColor = SunkTemperatureColor(max(shiftedTemperature, 900.0));
    thermalColor *= lerp(float3(1.0, 1.0, 1.0), float3(0.91, 1.02, 1.15), approachingWing);
    thermalColor *= lerp(float3(1.0, 1.0, 1.0), float3(1.13, 0.76, 0.55), recedingWing);
    return thermalColor * temperatureProfile * filaments * frequencyFlux *
        emission * edgeFade * middleValley * innerGlow;
}

#endif
