(module
  (type (;0;) (func (param i32 i32) (result i32)))
  (type (;1;) (func (param i32)))
  (import "env" "gas" (func (;0;) (type 1)))
  (func $add_locals (type 0) (param i32 i32) (result i32)
    (local i32)
    i32.const 5
    call 0
    local.get 0
    local.get 1
    call $add
    local.set 2
    local.get 2
  )
  (func $add (type 0) (param i32 i32) (result i32)
    i32.const 3
    call 0
    local.get 0
    local.get 1
    i32.add
  )
)