(module
  (type (;0;) (func (param i32 i32) (result i32)))
  (type (;1;) (func (param i32)))
  (import "env" "gas" (func $add_locals (type 1)))
  (func $add (type 0) (param $x i32) (param $y i32) (result i32)
    (local i32)
    i32.const 5
    call $add_locals
    local.get $x
    local.get $y
    call 2
    local.set 2
    local.get 2)
  (func (;2;) (type 0) (param i32 i32) (result i32)
    i32.const 3
    call $add_locals
    local.get 0
    local.get 1
    i32.add))