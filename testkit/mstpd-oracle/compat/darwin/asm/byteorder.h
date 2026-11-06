#pragma once
#include <linux/types.h>
#define __cpu_to_be16(x) ((__u16)__builtin_bswap16((__u16)(x)))
#define __be16_to_cpu(x) ((__u16)__builtin_bswap16((__u16)(x)))
#define __cpu_to_be32(x) ((__u32)__builtin_bswap32((__u32)(x)))
#define __be32_to_cpu(x) ((__u32)__builtin_bswap32((__u32)(x)))
#define __cpu_to_be64(x) ((__u64)__builtin_bswap64((__u64)(x)))
#define __be64_to_cpu(x) ((__u64)__builtin_bswap64((__u64)(x)))
#define __constant_cpu_to_be16(x) __cpu_to_be16(x)
#define __constant_cpu_to_be32(x) __cpu_to_be32(x)
#define __constant_be16_to_cpu(x) __be16_to_cpu(x)
#define __constant_be32_to_cpu(x) __be32_to_cpu(x)
