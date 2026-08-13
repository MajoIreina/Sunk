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
    float azimuthSign,
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
    float filaments = 0.82 + turbulence * (flow - 0.48) * 0.62;
    filaments *= 0.90 + 0.10 * sin(flowCoordinate.x * 9.0 - flowCoordinate.y * 5.0);

    float approaching = saturate(0.5 - 0.5 * azimuthSign);
    float doppler = lerp(1.0 - 0.42 * dopplerStrength, 1.0 + 0.78 * dopplerStrength, approaching);
    float3 spectralShift = lerp(float3(1.17, 0.63, 0.42), float3(0.91, 1.05, 1.20), approaching);

    float gravity = lerp(0.58, 1.0, smoothstep(0.0, 0.58, radial01));
    float3 redshift = lerp(float3(1.18, 0.46, 0.25), float3(1.0, 1.0, 1.0), gravity);
    redshift = lerp(float3(1.0, 1.0, 1.0), redshift, redshiftStrength);

    float edgeFade = smoothstep(0.0, 0.045, radial01) * (1.0 - smoothstep(0.80, 1.0, radial01));
    float thermalScale = saturate(diskTemperature / 12000.0);
    float3 thermalColor = lerp(
        SunkTemperatureColor(radial01) * float3(1.08, 0.62, 0.38),
        SunkTemperatureColor(radial01),
        thermalScale);
    return thermalColor * spectralShift * redshift *
        temperature * filaments * doppler * emission * edgeFade;
}

#endif
