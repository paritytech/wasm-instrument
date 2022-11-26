(module
  (type (;0;) (func (result i32)))
  (type (;1;) (func (param i64)))
  (func $fibonacci_with_break (;0;) (type 0) (result i32)
    (local $x i32) (local $y i32)
    i64.const 26
    call 1
    block ;; label = @1
      i32.const 0
      local.set $x
      i32.const 1
      local.set $y
      local.get $x
      local.get $y
      local.tee $x
      i32.add
      local.set $y
      i32.const 1
      br_if 0 (;@1;)
      i64.const 16
      call 1
      local.get $x
      local.get $y
      local.tee $x
      i32.add
      local.set $y
    end
    local.get $y
  )
  (func (;1;) (type 1) (param i64)
    global.get 0
    local.get 0
    i64.ge_u
    if ;; label = @1
      global.get 0
      local.get 0
      i64.sub
      global.set 0
    else
      i64.const -1
      global.set 0
      unreachable
    end
  )
  (global (;0;) (mut i64) i64.const 0)
  (export "gas_left" (global 0))
)