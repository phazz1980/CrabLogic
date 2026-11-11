(module
  (type (;0;) (func (param i32) (result i32)))
  (type (;1;) (func (param i32) (result f32)))
  (type (;2;) (func (param i32 i32)))
  (type (;3;) (func (param i32 f32)))
  (type (;4;) (func (result i32)))
  (type (;5;) (func (param i32)))
  (type (;6;) (func))
  (type (;7;) (func (param i32 i32) (result i32)))
  (type (;8;) (func (param i32 f32) (result f32)))
  (type (;9;) (func (param i32 i32 i32) (result f32)))
  (type (;10;) (func (param i32 i32 i32) (result i32)))
  (type (;11;) (func (param i32 i32 i32 i32 i32) (result i32)))
  (type (;12;) (func (param i32 i32 i32 i32) (result i32)))
  (type (;13;) (func (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (type (;14;) (func (param i32 f32 f32) (result f32)))
  (import "crab" "tag_get_i" (func (;0;) (type 0) (param i32) (result i32)))
  (import "crab" "tag_get_f" (func (;1;) (type 1) (param i32) (result f32)))
  (import "crab" "tag_set_i" (func (;2;) (type 2) (param i32 i32)))
  (import "crab" "tag_set_f" (func (;3;) (type 3) (param i32 f32)))
  (import "crab" "output_i" (func (;4;) (type 7) (param i32 i32) (result i32)))
  (import "crab" "output_f" (func (;5;) (type 8) (param i32 f32) (result f32)))
  (import "crab" "yield" (func (;6;) (type 4) (result i32)))
  (import "crab" "gpio_write" (func (;7;) (type 2) (param i32 i32)))
  (memory (;0;) 0)
  (global (;0;) (mut i32) i32.const 0)
  (global (;1;) (mut i32) i32.const 10000)
  (global (;2;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;3;) (mut i32) i32.const 0)
  (global (;4;) (mut i32) i32.const 0)
  (global (;5;) (mut i32) i32.const 0)
  (export "memory" (memory 0))
  (export "abi_version" (func 8))
  (export "step" (func 9))
  (export "init" (func 10))
  (func (;8;) (type 4) (result i32)
    i32.const 4
  )
  (func (;9;) (type 5) (param i32)
    global.get 1
    i32.const 1
    i32.sub
    global.set 1
    global.get 1
    i32.const 1
    i32.lt_s
    if ;; label = @1
      call 17
      global.set 1
    end
    global.get 0
    local.get 0
    i32.add
    global.set 0
    i32.const 0
    call 12
    global.set 2
    global.get 2
    f32.const 0x1.2p+2 (;=4.5;)
    f32.gt
    global.set 3
    global.get 3
    i32.const 0
    i32.ne
    global.set 4
    i32.const 2
    global.get 4
    call 18
    i32.const 1
    call 11
    i32.const 0
    i32.ne
    global.set 5
    i32.const 4
    global.get 5
    call 18
  )
  (func (;10;) (type 6)
    (local i32)
  )
  (func (;11;) (type 0) (param i32) (result i32)
    local.get 0
    call 0
  )
  (func (;12;) (type 1) (param i32) (result f32)
    local.get 0
    call 1
  )
  (func (;13;) (type 2) (param i32 i32)
    local.get 0
    local.get 1
    call 2
  )
  (func (;14;) (type 3) (param i32 f32)
    local.get 0
    local.get 1
    call 3
  )
  (func (;15;) (type 7) (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call 4
  )
  (func (;16;) (type 8) (param i32 f32) (result f32)
    local.get 0
    local.get 1
    call 5
  )
  (func (;17;) (type 4) (result i32)
    call 6
  )
  (func (;18;) (type 2) (param i32 i32)
    local.get 0
    local.get 1
    call 7
  )
  (@custom "crab.mem" (after code) "\00\00\00\00")
)
