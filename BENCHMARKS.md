# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
    - [Coremark, instrumented](#coremark,-instrumented)
    - [recursive_ok, instrumented](#recursive_ok,-instrumented)
    - [fibonacci_recursive, instrumented](#fibonacci_recursive,-instrumented)
    - [factorial_recursive, instrumented](#factorial_recursive,-instrumented)
    - [count_until, instrumented](#count_until,-instrumented)
    - [memory_vec_add, instrumented](#memory_vec_add,-instrumented)

## Instrumented Modules sizes

| fixture                      |  original size   | gas metered/host fn | gas metered/mut global | size diff |
|------------------------------|------------------|---------------------|------------------------|-----------|
| many_blocks.wasm             |          1023 kb |      2389 kb (233%) |         3754 kb (366%) |      +57% |
| recursive_ok.wat             |             0 kb |         0 kb (137%) |            0 kb (196%) |      +42% |
| count_until.wat              |             0 kb |         0 kb (125%) |            0 kb (166%) |      +31% |
| factorial.wat                |             0 kb |         0 kb (125%) |            0 kb (162%) |      +29% |
| global_bump.wat              |             0 kb |         0 kb (123%) |            0 kb (156%) |      +27% |
| memory-vec-add.wat           |             0 kb |         0 kb (116%) |            0 kb (142%) |      +22% |
| fibonacci.wat                |             0 kb |         0 kb (121%) |            0 kb (148%) |      +22% |
| coremark_minimal.wasm        |             7 kb |         8 kb (114%) |            9 kb (130%) |      +13% |
| contract_transfer.wasm       |             7 kb |         8 kb (113%) |            9 kb (126%) |      +11% |
| contract_terminate.wasm      |             1 kb |         1 kb (110%) |            1 kb (121%) |      +10% |
| multisig.wasm                |            27 kb |        30 kb (110%) |           33 kb (120%) |       +9% |
| erc1155.wasm                 |            26 kb |        29 kb (111%) |           32 kb (122%) |       +9% |
| rand_extension.wasm          |             4 kb |         5 kb (109%) |            5 kb (118%) |       +8% |
| erc20.wasm                   |             9 kb |        10 kb (108%) |           11 kb (117%) |       +8% |
| proxy.wasm                   |             3 kb |         4 kb (108%) |            4 kb (118%) |       +8% |
| trait_erc20.wasm             |            10 kb |        11 kb (108%) |           12 kb (117%) |       +7% |
| dns.wasm                     |            10 kb |        11 kb (108%) |           11 kb (116%) |       +7% |
| erc721.wasm                  |            13 kb |        14 kb (108%) |           15 kb (117%) |       +7% |
| wasm_kernel.wasm             |           779 kb |       787 kb (100%) |          825 kb (105%) |       +4% |

## Benchmark Results

### coremark, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `16.54 s` (✅ **1.00x**)                 | `19.02 s` (❌ *1.15x slower*)              |

### recursive_ok, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `425.87 us` (✅ **1.00x**)               | `627.48 us` (❌ *1.47x slower*)            |

### fibonacci_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `10.51 us` (✅ **1.00x**)                | `14.57 us` (❌ *1.39x slower*)             |

### factorial_recursive, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `1.45 us` (✅ **1.00x**)                 | `2.15 us` (❌ *1.48x slower*)              |

### count_until, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `4.75 ms` (✅ **1.00x**)                 | `8.34 ms` (❌ *1.75x slower*)              |

### memory_vec_add, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `7.37 ms` (✅ **1.00x**)                 | `11.31 ms` (❌ *1.54x slower*)             |

### global_bump, instrumented

|        | `with host_function::Injector`          | `with mutable_global::Injector`           |
|:-------|:----------------------------------------|:----------------------------------------- |
|        | `3.47 ms` (✅ **1.00x**)                 | `7.76 ms` (❌ *2.24x slower*)              |

---
Made with [criterion-table](https://github.com/nu11ptr/criterion-table)

