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

## Instrumented Modules sizes

| fixture                      |  original size   | gas metered/host fn | gas metered/mut global | size diff |
|------------------------------|------------------|---------------------|------------------------|-----------|
| many_blocks.wasm             |          1023 kb |      2389 kb (233%) |         4778 kb (466%) |      +99% |
| recursive_ok.wat             |             0 kb |         0 kb (137%) |            0 kb (207%) |      +50% |
| count_until.wat              |             0 kb |         0 kb (125%) |            0 kb (174%) |      +38% |
| factorial.wat                |             0 kb |         0 kb (125%) |            0 kb (173%) |      +38% |
| global_bump.wat              |             0 kb |         0 kb (123%) |            0 kb (164%) |      +33% |
| fibonacci.wat                |             0 kb |         0 kb (121%) |            0 kb (159%) |      +31% |
| memory-vec-add.wat           |             0 kb |         0 kb (116%) |            0 kb (147%) |      +26% |
| coremark_minimal.wasm        |             7 kb |         8 kb (114%) |           10 kb (140%) |      +22% |
| contract_transfer.wasm       |             7 kb |         8 kb (113%) |           10 kb (136%) |      +20% |
| erc1155.wasm                 |            26 kb |        29 kb (111%) |           34 kb (130%) |      +17% |
| multisig.wasm                |            27 kb |        30 kb (110%) |           35 kb (128%) |      +16% |
| contract_terminate.wasm      |             1 kb |         1 kb (110%) |            1 kb (128%) |      +16% |
| rand_extension.wasm          |             4 kb |         5 kb (109%) |            6 kb (125%) |      +14% |
| proxy.wasm                   |             3 kb |         4 kb (108%) |            4 kb (124%) |      +14% |
| trait_erc20.wasm             |            10 kb |        11 kb (108%) |           12 kb (123%) |      +13% |
| erc20.wasm                   |             9 kb |        10 kb (108%) |           12 kb (123%) |      +13% |
| dns.wasm                     |            10 kb |        11 kb (108%) |           12 kb (122%) |      +13% |
| erc721.wasm                  |            13 kb |        14 kb (108%) |           16 kb (123%) |      +13% |
| wasm_kernel.wasm             |           779 kb |       787 kb (100%) |          847 kb (108%) |       +7% |


## Benchmark Results

### Coremark, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `19.16 s` (✅ **1.00x**)                 | `17.23 s` (✅ **1.11x faster**)            |

### recursive_ok, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `408.70 us` (✅ **1.00x**)               | `742.90 us` (❌ *1.82x slower*)            |

### fibonacci_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `11.70 us` (✅ **1.00x**)                | `19.63 us` (❌ *1.68x slower*)             |

### factorial_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `1.57 us` (✅ **1.00x**)                 | `2.54 us` (❌ *1.62x slower*)              |

### count_until, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `5.60 ms` (✅ **1.00x**)                 | `10.35 ms` (❌ *1.85x slower*)             |

### memory_vec_add, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `8.77 ms` (✅ **1.00x**)                 | `13.18 ms` (❌ *1.50x slower*)             |

### wasm_kernel::tiny_keccak, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `1.32 ms` (✅ **1.00x**)                 | `1.53 ms` (❌ *1.16x slower*)              |

### global_bump, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `4.15 ms` (✅ **1.00x**)                 | `8.92 ms` (❌ *2.15x slower*)              |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

