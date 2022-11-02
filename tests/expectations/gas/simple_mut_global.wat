(module
  (type (;0;) (func))
  (type (;1;) (func (param i64)))
  (func (;0;) (type 0)
    i64.const 2
    call 2
    i32.const 1
    if  ;; label = @1
      i64.const 1
      call 2
      loop  ;; label = @2
        i64.const 2
        call 2
        i32.const 123
        drop
      end
    end
  )
  (func (;1;) (type 0)
    i64.const 1
    call 2
    block  ;; label = @1
    end
  )
  (func (;2;) (type 1) (param i64)
    (local i64)
    global.get 0
    i64.const 13
    i64.sub
    local.tee 1
    local.get 0
    i64.ge_u
    if  ;; label = @1
      local.get 1
      local.get 0
      i64.sub
      global.set 0
      return
    end
    i64.const -1
    global.set 0
    global.set 0
    unreachable
  )
  (global (;0;) (mut i64) i64.const 0)
  (export "simple" (func 0))
  (export "gas_left" (global 0))
)