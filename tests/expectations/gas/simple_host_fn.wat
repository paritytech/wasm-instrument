(module
  (type (;0;) (func))
  (type (;1;) (func (param i64)))
  (import "env" "gas" (func (;0;) (type 1)))
  (func (;1;) (type 0)
    i64.const 2
    call 0
    i32.const 1
    if ;; label = @1
      i64.const 1
      call 0
      loop ;; label = @2
        i64.const 2
        call 0
        i32.const 123
        drop
      end
    end
  )
  (func (;2;) (type 0)
    i64.const 1
    call 0
    block ;; label = @1
    end
  )
  (export "simple" (func 1))
)