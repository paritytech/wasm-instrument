# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
    - [Coremark, instrumented](#coremark,-instrumented)
    - [recursive_ok, instrumented](#recursive_ok,-instrumented)
    - [fibonacci_recursive, instrumented](#fibonacci_recursive,-instrumented)
    - [factorial_recursive, instrumented](#factorial_recursive,-instrumented)
    - [count_until, instrumented](#count_until,-instrumented)
    - [memory_vec_add, instrumented](#memory_vec_add,-instrumented)
    - [wasm_kernel::tiny_keccak, instrumented](#wasm_kernel::tiny_keccak,-instrumented)
    - [global_bump, instrumented](#global_bump,-instrumented)

## Benchmark Results

### Coremark, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `18.83 s` (✅ **1.00x**)                 | `17.84 s` (✅ **1.06x faster**)            |

### recursive_ok, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `375.29 us` (✅ **1.00x**)               | `698.74 us` (❌ *1.86x slower*)            |

### fibonacci_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `10.18 us` (✅ **1.00x**)                | `17.88 us` (❌ *1.76x slower*)             |

### factorial_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `1.36 us` (✅ **1.00x**)                 | `2.54 us` (❌ *1.86x slower*)              |

### count_until, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `5.18 ms` (✅ **1.00x**)                 | `9.83 ms` (❌ *1.90x slower*)              |

### memory_vec_add, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `8.73 ms` (✅ **1.00x**)                 | `13.53 ms` (❌ *1.55x slower*)             |

### wasm_kernel::tiny_keccak, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `1.35 ms` (✅ **1.00x**)                 | `1.59 ms` (❌ *1.18x slower*)              |

### global_bump, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `3.94 ms` (✅ **1.00x**)                 | `9.29 ms` (❌ *2.36x slower*)              |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

