#ifndef SUNK_GARGANTUA_DISK_INCLUDED
#define SUNK_GARGANTUA_DISK_INCLUDED

float3 SunkTemperatureColor(float radial01)
{
    float3 hot = float3(2.35, 2.05, 1.72);
    float3 gold = float3(1.90, 0.76, 0.18);
    float3 ember = float3(0.72, 0.085, 0.012);
    float innerBlend = smoothstep(0.0, 0.48, radial01);
    float outerBlend = smoothstep(0.42, 1.0, radial01);
    return lerp(lerp(hot, gold, innerBlend), ember, outerBlend);
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
    float temperature = pow(max(innerRadius / max(diskRadius, innerRadius), 0.001), 0.75);
    float flow = SunkFbm(flowCoordinate + float2(_Time.y * rotationSpeed, -_Time.y * rotationSpeed * 0.31));
    float filaments = 0.80 + turbulence * (flow - 0.46) * 0.72;
    filaments *= 0.88 + 0.12 * sin(flowCoordinate.x * 10.0 - flowCoordinate.y * 5.0);

    float approaching = saturate(0.5 + 0.5 * orbitalToObserver);
    float beaming = clamp(dopplerFactor * dopplerFactor * dopplerFactor, 0.28, 3.25);
    float doppler = lerp(1.0, beaming, dopplerStrength);
    float observedEnergy = gravityShift * lerp(1.0, dopplerFactor, dopplerStrength);
    float blueShift = smoothstep(0.92, 1.42, observedEnergy);
    float redShift = 1.0 - smoothstep(0.58, 1.0, observedEnergy);
    float3 spectralShift = lerp(float3(1.0, 0.88, 0.78), float3(0.86, 1.04, 1.22), blueShift);
    spectralShift *= lerp(float3(1.18, 0.55, 0.30), float3(1.0, 1.0, 1.0), 1.0 - redShift);

    float gravityFlux = lerp(1.0, gravityShift * gravityShift, redshiftStrength);
    float3 gravitationalTint = lerp(
        float3(1.20, 0.48, 0.24),
        float3(1.0, 1.0, 1.0),
        lerp(1.0, gravityShift, redshiftStrength));

    float edgeFade = smoothstep(0.0, 0.045, radial01) * (1.0 - smoothstep(0.80, 1.0, radial01));
    float thermalScale = saturate(diskTemperature / 12000.0);
    float3 thermalColor = lerp(
        SunkTemperatureColor(radial01) * float3(1.08, 0.62, 0.38),
        SunkTemperatureColor(radial01),
        thermalScale);
    return thermalColor * spectralShift * gravitationalTint *
        temperature * filaments * doppler * gravityFlux * emission * edgeFade;
}

#endif
