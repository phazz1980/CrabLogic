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
  (import "crab" "uart" (func (;7;) (type 12) (param i32 i32 i32 i32) (result i32)))
  (import "crab" "text" (func (;8;) (type 13) (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (memory (;0;) 1)
  (global (;0;) (mut i32) i32.const 0)
  (global (;1;) (mut i32) i32.const 10000)
  (global (;2;) (mut i32) i32.const 0)
  (global (;3;) (mut i32) i32.const 0)
  (global (;4;) (mut i32) i32.const 0)
  (global (;5;) (mut i32) i32.const 0)
  (global (;6;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;7;) (mut i32) i32.const 0)
  (global (;8;) (mut i32) i32.const 0)
  (global (;9;) (mut i32) i32.const 0)
  (global (;10;) (mut i32) i32.const 0)
  (global (;11;) (mut i32) i32.const 0)
  (global (;12;) (mut i32) i32.const 0)
  (global (;13;) (mut i32) i32.const 0)
  (global (;14;) (mut i32) i32.const 0)
  (global (;15;) (mut i32) i32.const 0)
  (global (;16;) (mut i32) i32.const 0)
  (global (;17;) (mut i32) i32.const 0)
  (global (;18;) (mut i32) i32.const 0)
  (global (;19;) (mut i32) i32.const 0)
  (global (;20;) (mut i32) i32.const 0)
  (global (;21;) (mut i32) i32.const 0)
  (export "memory" (memory 0))
  (export "abi_version" (func 9))
  (export "step" (func 10))
  (export "init" (func 11))
  (func (;9;) (type 4) (result i32)
    i32.const 4
  )
  (func (;10;) (type 5) (param i32)
    (local i32 i32 i32 i32 f32)
    global.get 1
    i32.const 1
    i32.sub
    global.set 1
    global.get 1
    i32.const 1
    i32.lt_s
    if ;; label = @1
      call 6
      global.set 1
    end
    global.get 0
    local.get 0
    i32.add
    global.set 0
    global.get 14
    i32.eqz
    if ;; label = @1
      i32.const 0
      i32.const 2
      i32.const 0
      i32.const 0
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 1
      i32.const 115200
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 2
      i32.const 8
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 3
      i32.const 0
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 4
      i32.const 1
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 5
      i32.const 16
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 6
      i32.const 17
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 7
      i32.const -1
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 8
      i32.const 256
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 63
      i32.const 0
      call 7
      drop
      i32.const 1
      global.set 14
    end
    i32.const 0
    local.set 1
    block ;; label = @1
      loop ;; label = @2
        global.get 1
        i32.const 1
        i32.sub
        global.set 1
        global.get 1
        i32.const 1
        i32.lt_s
        if ;; label = @3
          call 6
          global.set 1
        end
        i32.const 2
        i32.const 2
        i32.const 0
        i32.const 0
        call 7
        i32.const 0
        i32.le_s
        br_if 1 (;@1;)
        i32.const 1
        i32.const 2
        i32.const 0
        i32.const 0
        call 7
        local.set 2
        local.get 2
        i32.const 13
        i32.eq
        br_if 0 (;@2;)
        local.get 2
        i32.const 10
        i32.eq
        local.set 3
        local.get 3
        i32.eqz
        if ;; label = @3
          i32.const 4
          i32.const 304
          i32.const 32
          local.get 2
          i32.const 0
          global.get 15
          i32.const 0
          call 8
          global.set 15
          global.get 15
          i32.const 32
          i32.ge_s
          local.set 3
        end
        local.get 3
        if ;; label = @3
          i32.const 1
          i32.const 16
          i32.const 32
          i32.const 304
          global.get 15
          i32.const 0
          i32.const 0
          call 8
          global.set 3
          i32.const 0
          global.set 15
          i32.const 1
          local.set 1
          br 2 (;@1;)
        end
        br 0 (;@2;)
      end
    end
    local.get 1
    global.set 4
    i32.const 2
    i32.const 2
    i32.const 0
    i32.const 0
    call 7
    global.set 2
    i32.const 6
    i32.const 0
    i32.const 4
    i32.const 16
    global.get 3
    i32.const 0
    i32.const 0
    call 8
    local.set 1
    local.get 1
    if ;; label = @1
      i32.const 0
      f32.load
      global.set 6
    end
    local.get 1
    global.set 5
    i32.const 5
    i32.const 0
    i32.const 1
    i32.const 16
    global.get 3
    i32.const 336
    i32.const 2
    call 8
    global.set 7
    global.get 6
    local.set 5
    i32.const 1
    i32.const 48
    i32.const 128
    i32.const 340
    i32.const 2
    i32.const 0
    i32.const 0
    call 8
    local.set 1
    i32.const 2
    i32.const 48
    i32.const 128
    local.get 5
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    call 8
    local.set 1
    i32.const 1
    i32.const 48
    i32.const 128
    i32.const 344
    i32.const 1
    local.get 1
    i32.const 0
    call 8
    local.set 1
    local.get 1
    global.set 8
    i32.const 1
    i32.const 176
    i32.const 128
    i32.const 348
    i32.const 15
    i32.const 0
    i32.const 0
    call 8
    local.set 1
    i32.const 1
    i32.const 176
    i32.const 128
    i32.const 16
    global.get 3
    local.get 1
    i32.const 0
    call 8
    local.set 1
    local.get 1
    global.set 9
    global.get 17
    i32.eqz
    if ;; label = @1
      i32.const 0
      i32.const 2
      i32.const 0
      i32.const 0
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 1
      i32.const 115200
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 2
      i32.const 8
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 3
      i32.const 0
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 4
      i32.const 1
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 5
      i32.const 16
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 6
      i32.const 17
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 7
      i32.const -1
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 8
      i32.const 256
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 63
      i32.const 0
      call 7
      drop
      i32.const 1
      global.set 17
    end
    global.get 4
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    local.set 1
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 364
      i32.const 130
      i32.const 48
      global.get 8
      i32.const 0
      i32.const 0
      call 8
      local.set 4
      i32.const 1
      i32.const 364
      i32.const 130
      i32.const 496
      i32.const 2
      local.get 4
      i32.const 0
      call 8
      local.set 4
      i32.const 6
      i32.const 2
      i32.const 364
      local.get 4
      call 7
      local.set 3
    end
    local.get 3
    global.set 10
    global.get 3
    global.set 11
    global.get 20
    i32.eqz
    if ;; label = @1
      i32.const 0
      i32.const 2
      i32.const 0
      i32.const 0
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 1
      i32.const 115200
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 2
      i32.const 8
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 3
      i32.const 0
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 4
      i32.const 1
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 5
      i32.const 16
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 6
      i32.const 17
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 7
      i32.const -1
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 8
      i32.const 256
      call 7
      drop
      i32.const 0
      i32.const 2
      i32.const 63
      i32.const 0
      call 7
      drop
      i32.const 1
      global.set 20
    end
    i32.const 0
    i32.const 0
    i32.ne
    local.set 1
    local.get 1
    global.get 21
    i32.eqz
    i32.and
    if ;; label = @1
      i32.const 1
      local.set 2
      i32.const 5
      i32.const 2
      i32.const 115200
      i32.const 0
      call 7
      drop
      local.get 2
      global.set 13
    end
    local.get 1
    global.set 21
    i32.const 4
    i32.const 2
    i32.const 0
    i32.const 0
    call 7
    global.set 12
  )
  (func (;11;) (type 6)
    (local i32)
  )
  (data (;0;) (i32.const 336) "on")
  (data (;1;) (i32.const 340) "T=")
  (data (;2;) (i32.const 344) "C")
  (data (;3;) (i32.const 348) "\d0\ba\d0\be\d0\bc\d0\b0\d0\bd\d0\b4\d0\b0 ")
  (data (;4;) (i32.const 496) "\0d\0a")
  (@custom "crab.mem" (after data) "@\02\00\00")
)
