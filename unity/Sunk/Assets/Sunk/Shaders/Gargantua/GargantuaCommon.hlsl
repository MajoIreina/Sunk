#ifndef SUNK_GARGANTUA_COMMON_INCLUDED
#define SUNK_GARGANTUA_COMMON_INCLUDED

static const float SUNK_PI = 3.14159265359;

float SunkHash12(float2 value)
{
    float3 p = frac(float3(value.xyx) * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return frac((p.x + p.y) * p.z);
}

float SunkNoise(float2 value)
{
    float2 cell = floor(value);
    float2 local = frac(value);
    local = local * local * (3.0 - 2.0 * local);

    float a = SunkHash12(cell);
    float b = SunkHash12(cell + float2(1.0, 0.0));
    float c = SunkHash12(cell + float2(0.0, 1.0));
    float d = SunkHash12(cell + float2(1.0, 1.0));
    return lerp(lerp(a, b, local.x), lerp(c, d, local.x), local.y);
}

float SunkFbm(float2 value)
{
    float result = 0.0;
    float amplitude = 0.55;
    result += amplitude * SunkNoise(value);
    value = value * 2.03 + 17.17;
    amplitude *= 0.5;
    result += amplitude * SunkNoise(value);
    value = value * 2.11 - 9.41;
    amplitude *= 0.5;
    result += amplitude * SunkNoise(value);
    return result;
}

float SunkGaussian(float value, float width)
{
    float normalized = value / max(width, 0.0001);
    return exp2(-normalized * normalized * 1.442695);
}

#endif
