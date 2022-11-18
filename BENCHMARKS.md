# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
    - [coremark, instrumented](#coremark,-instrumented)
    - [recursive_ok, instrumented](#recursive_ok,-instrumented)
    - [fibonacci_recursive, instrumented](#fibonacci_recursive,-instrumented)
    - [factorial_recursive, instrumented](#factorial_recursive,-instrumented)
    - [count_until, instrumented](#count_until,-instrumented)
    - [memory_vec_add, instrumented](#memory_vec_add,-instrumented)
    - [wasm_kernel::tiny_keccak, instrumented](#wasm_kernel::tiny_keccak,-instrumented)
    - [global_bump, instrumented](#global_bump,-instrumented)

## Instrumented Modules sizes

| fixture                      |  original size   | gas metered/host fn | gas metered/mut global | size diff |
|------------------------------|------------------|---------------------|------------------------|-----------|
| recursive_ok.wat             |             0 kb |         0 kb (137%) |            0 kb (177%) |      +29% |
| count_until.wat              |             0 kb |         0 kb (125%) |            0 kb (153%) |      +21% |
| global_bump.wat              |             0 kb |         0 kb (123%) |            0 kb (145%) |      +18% |
| memory-vec-add.wat           |             0 kb |         0 kb (116%) |            0 kb (134%) |      +15% |
| factorial.wat                |             0 kb |         0 kb (125%) |            0 kb (145%) |      +15% |
| fibonacci.wat                |             0 kb |         0 kb (121%) |            0 kb (134%) |      +10% |
| contract_terminate.wasm      |             1 kb |         1 kb (110%) |            1 kb (112%) |       +2% |
| coremark_minimal.wasm        |             7 kb |         8 kb (114%) |            8 kb (115%) |       +0% |
| trait_erc20.wasm             |            10 kb |        11 kb (108%) |           11 kb (108%) |       +0% |
| rand_extension.wasm          |             4 kb |         5 kb (109%) |            5 kb (109%) |       +0% |
| multisig.wasm                |            27 kb |        30 kb (110%) |           30 kb (110%) |       +0% |
| wasm_kernel.wasm             |           779 kb |       787 kb (100%) |          795 kb (101%) |       +0% |
| many_blocks.wasm             |          1023 kb |      2389 kb (233%) |         2389 kb (233%) |       +0% |
| contract_transfer.wasm       |             7 kb |         8 kb (113%) |            8 kb (113%) |       +0% |
| erc1155.wasm                 |            26 kb |        29 kb (111%) |           29 kb (111%) |       +0% |
| erc20.wasm                   |             9 kb |        10 kb (108%) |           10 kb (109%) |       +0% |
| dns.wasm                     |            10 kb |        11 kb (108%) |           11 kb (108%) |       +0% |
| proxy.wasm                   |             3 kb |         4 kb (108%) |            4 kb (109%) |       +0% |
| erc721.wasm                  |            13 kb |        14 kb (108%) |           14 kb (108%) |       +0% |

## Benchmark Results

### coremark, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `20.81 s` (✅ **1.00x**)                 | `20.20 s` (✅ **1.03x faster**)            |

### recursive_ok, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `367.11 us` (✅ **1.00x**)               | `585.39 us` (❌ *1.59x slower*)            |

### fibonacci_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `9.15 us` (✅ **1.00x**)                 | `13.56 us` (❌ *1.48x slower*)             |

### factorial_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `1.50 us` (✅ **1.00x**)                 | `1.98 us` (❌ *1.32x slower*)              |

### count_until, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `5.03 ms` (✅ **1.00x**)                 | `8.13 ms` (❌ *1.62x slower*)              |

### memory_vec_add, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `6.21 ms` (✅ **1.00x**)                 | `8.45 ms` (❌ *1.36x slower*)              |

### wasm_kernel::tiny_keccak, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `925.22 us` (✅ **1.00x**)               | `1.08 ms` (❌ *1.17x slower*)              |

### global_bump, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `3.79 ms` (✅ **1.00x**)                 | `7.03 ms` (❌ *1.86x slower*)              |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

