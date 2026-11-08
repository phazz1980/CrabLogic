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
  (import "crab" "uart" (func (;6;) (type 12) (param i32 i32 i32 i32) (result i32)))
  (import "crab" "text" (func (;7;) (type 13) (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (import "crab" "math" (func (;8;) (type 14) (param i32 f32 f32) (result f32)))
  (memory (;0;) 1)
  (global (;0;) (mut i32) i32.const 0)
  (global (;1;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;2;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;3;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;4;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;5;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;6;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;7;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;8;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;9;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;10;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;11;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;12;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;13;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;14;) (mut i32) i32.const 0)
  (global (;15;) (mut i32) i32.const 0)
  (global (;16;) (mut i32) i32.const 0)
  (global (;17;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;18;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;19;) (mut i32) i32.const 0)
  (global (;20;) (mut i32) i32.const 0)
  (global (;21;) (mut i32) i32.const 0)
  (global (;22;) (mut i32) i32.const 0)
  (global (;23;) (mut i32) i32.const 0)
  (global (;24;) (mut i32) i32.const 0)
  (global (;25;) (mut i32) i32.const 0)
  (global (;26;) (mut i32) i32.const 0)
  (global (;27;) (mut i32) i32.const 0)
  (global (;28;) (mut i32) i32.const 0)
  (global (;29;) (mut i32) i32.const 0)
  (global (;30;) (mut i32) i32.const 0)
  (global (;31;) (mut i32) i32.const 0)
  (global (;32;) (mut i32) i32.const 0)
  (global (;33;) (mut i32) i32.const 0)
  (global (;34;) (mut i32) i32.const 0)
  (global (;35;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;36;) (mut i32) i32.const 0)
  (global (;37;) (mut i32) i32.const 0)
  (global (;38;) (mut i32) i32.const 0)
  (global (;39;) (mut i32) i32.const 0)
  (global (;40;) (mut i32) i32.const 0)
  (global (;41;) (mut i32) i32.const 0)
  (global (;42;) (mut i32) i32.const 0)
  (global (;43;) (mut i32) i32.const 0)
  (global (;44;) (mut i32) i32.const 0)
  (global (;45;) (mut i32) i32.const 0)
  (global (;46;) (mut i32) i32.const 0)
  (global (;47;) (mut i32) i32.const 0)
  (global (;48;) (mut i32) i32.const 0)
  (global (;49;) (mut i32) i32.const 0)
  (global (;50;) (mut i32) i32.const 0)
  (global (;51;) (mut i32) i32.const 0)
  (global (;52;) (mut i32) i32.const 0)
  (global (;53;) (mut i32) i32.const 0)
  (global (;54;) (mut i32) i32.const 0)
  (global (;55;) (mut i32) i32.const 0)
  (global (;56;) (mut i32) i32.const 0)
  (global (;57;) (mut i32) i32.const 0)
  (global (;58;) (mut i32) i32.const 0)
  (global (;59;) (mut i32) i32.const 0)
  (global (;60;) (mut i32) i32.const 0)
  (global (;61;) (mut i32) i32.const 0)
  (global (;62;) (mut i32) i32.const 0)
  (global (;63;) (mut i32) i32.const 0)
  (global (;64;) (mut i32) i32.const 0)
  (global (;65;) (mut i32) i32.const 0)
  (export "memory" (memory 0))
  (export "abi_version" (func 9))
  (export "step" (func 10))
  (export "init" (func 11))
  (func (;9;) (type 4) (result i32)
    i32.const 3
  )
  (func (;10;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 0
    local.get 0
    i32.add
    global.set 0
    f32.const -0x1.cp+1 (;=-3.5;)
    f32.abs
    global.set 1
    f32.const 0x1p+1 (;=2;)
    f32.const 0x1.4p+2 (;=5;)
    local.set 8
    local.set 7
    local.get 7
    local.get 8
    local.get 7
    local.get 8
    f32.lt
    select
    global.set 2
    f32.const 0x1p+1 (;=2;)
    f32.const 0x1.4p+2 (;=5;)
    local.set 8
    local.set 7
    local.get 7
    local.get 8
    local.get 7
    local.get 8
    f32.gt
    select
    global.set 3
    f32.const 0x0p+0 (;=0;)
    f32.const 0x1.9p+6 (;=100;)
    local.set 10
    local.set 9
    local.get 9
    local.get 10
    local.get 9
    local.get 10
    f32.lt
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    f32.const 0x1.9p+6 (;=100;)
    local.set 12
    local.set 11
    local.get 11
    local.get 12
    local.get 11
    local.get 12
    f32.gt
    select
    local.set 8
    f32.const 0x1.2cp+7 (;=150;)
    local.get 7
    local.set 14
    local.set 13
    local.get 13
    local.get 14
    local.get 13
    local.get 14
    f32.gt
    select
    local.get 8
    local.set 16
    local.set 15
    local.get 15
    local.get 16
    local.get 15
    local.get 16
    f32.lt
    select
    global.set 4
    f32.const 0x1p+4 (;=16;)
    local.tee 7
    f32.const 0x0p+0 (;=0;)
    f32.gt
    if (result f32) ;; label = @1
      local.get 7
      f32.sqrt
    else
      f32.const 0x0p+0 (;=0;)
    end
    global.set 5
    f32.const -0x1.8p+1 (;=-3;)
    local.tee 7
    local.get 7
    f32.mul
    global.set 6
    f32.const 0x1p+1 (;=2;)
    local.set 7
    f32.const 0x1.4p+3 (;=10;)
    local.set 8
    i32.const 3
    local.get 7
    local.get 8
    call 8
    global.set 7
    f32.const 0x1.1p+4 (;=17;)
    local.set 7
    f32.const 0x1.4p+2 (;=5;)
    local.set 8
    local.get 8
    f32.const 0x0p+0 (;=0;)
    f32.ne
    if (result f32) ;; label = @1
      local.get 7
      f64.promote_f32
      local.get 8
      f64.promote_f32
      local.get 7
      f64.promote_f32
      local.get 8
      f64.promote_f32
      f64.div
      f64.trunc
      f64.mul
      f64.sub
      local.get 7
      f64.promote_f32
      f64.copysign
      f32.demote_f64
    else
      f32.const 0x0p+0 (;=0;)
    end
    global.set 8
    local.get 0
    call 12
    local.get 0
    call 13
    local.get 0
    call 14
    local.get 0
    call 15
    local.get 0
    call 16
  )
  (func (;11;) (type 6)
    (local i32)
  )
  (func (;12;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32 f32 f32)
    f32.const 0x1p+1 (;=2;)
    f32.const 0x1p+0 (;=1;)
    i32.const 1
    i32.const 0
    i32.ne
    select
    global.set 9
    f32.const 0x1p+1 (;=2;)
    i32.trunc_sat_f32_s
    i32.const 0
    local.set 3
    local.set 2
    local.get 2
    local.get 3
    local.get 2
    local.get 3
    i32.gt_s
    select
    local.set 1
    f32.const 0x1.ep+4 (;=30;)
    local.set 7
    f32.const 0x1.4p+4 (;=20;)
    local.get 7
    local.get 1
    i32.const 2
    i32.lt_s
    select
    local.set 7
    f32.const 0x1.4p+3 (;=10;)
    local.get 7
    local.get 1
    i32.const 1
    i32.lt_s
    select
    local.set 7
    local.get 7
    global.set 10
    f32.const 0x1p+0 (;=1;)
    i32.trunc_sat_f32_s
    local.set 1
    f32.const 0x1.cp+2 (;=7;)
    local.set 7
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 0
    i32.eq
    select
    global.set 11
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 1
    i32.eq
    select
    global.set 12
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 2
    i32.eq
    select
    global.set 13
    i32.const 1
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      i32.const 700
      local.set 1
      i32.const 700
      local.set 2
      global.get 47
      if ;; label = @2
        i32.const 0
        global.set 47
        i32.const 0
        global.set 46
        global.get 0
        global.set 45
      end
      local.get 2
      local.get 1
      global.get 46
      select
      local.set 3
      global.get 0
      global.get 45
      i32.sub
      local.set 4
      local.get 4
      local.get 3
      i32.ge_u
      if ;; label = @2
        local.get 4
        local.get 3
        i32.sub
        local.get 1
        local.get 2
        i32.add
        local.get 3
        i32.sub
        i32.lt_u
        if ;; label = @3
          global.get 46
          i32.const 1
          i32.xor
          global.set 46
          global.get 45
          local.get 3
          i32.add
          global.set 45
        else
          local.get 1
          local.get 2
          i32.add
          local.set 6
          i32.const 1
          local.get 6
          local.get 6
          i32.eqz
          select
          local.set 6
          local.get 4
          local.get 6
          i32.rem_u
          local.set 5
          local.get 5
          local.get 3
          i32.lt_u
          if ;; label = @4
            global.get 0
            local.get 5
            i32.sub
            global.set 45
          else
            global.get 46
            i32.const 1
            i32.xor
            global.set 46
            global.get 0
            local.get 5
            local.get 3
            i32.sub
            i32.sub
            global.set 45
          end
        end
      end
      global.get 46
      i32.eqz
      local.get 1
      i32.const 0
      i32.ne
      i32.and
    else
      i32.const 1
      global.set 47
      i32.const 0
    end
    global.set 14
    i32.const 0
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      i32.const 0
    else
      global.get 14
      i32.const 0
      i32.ne
      global.get 48
      i32.eqz
      i32.and
      if (result i32) ;; label = @2
        global.get 15
        i32.eqz
      else
        global.get 15
      end
    end
    global.get 14
    i32.const 0
    i32.ne
    global.set 48
    global.set 15
    global.get 14
    i32.const 0
    i32.ne
    local.set 1
    local.get 1
    global.get 49
    i32.ne
    if ;; label = @1
      local.get 1
      global.set 49
      global.get 0
      global.set 50
    end
    global.get 0
    global.get 50
    i32.sub
    i32.const 100
    i32.ge_u
    if (result i32) ;; label = @1
      global.get 49
    else
      global.get 16
    end
    global.set 16
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 15
    i32.const 0
    i32.ne
    select
    global.set 17
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 16
    i32.const 0
    i32.ne
    select
    global.set 18
  )
  (func (;13;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32 f32 f32)
    i32.const 1
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      i32.const 100
      local.set 1
      i32.const 900
      local.set 2
      global.get 53
      if ;; label = @2
        i32.const 0
        global.set 53
        i32.const 0
        global.set 52
        global.get 0
        global.set 51
      end
      local.get 2
      local.get 1
      global.get 52
      select
      local.set 3
      global.get 0
      global.get 51
      i32.sub
      local.set 4
      local.get 4
      local.get 3
      i32.ge_u
      if ;; label = @2
        local.get 4
        local.get 3
        i32.sub
        local.get 1
        local.get 2
        i32.add
        local.get 3
        i32.sub
        i32.lt_u
        if ;; label = @3
          global.get 52
          i32.const 1
          i32.xor
          global.set 52
          global.get 51
          local.get 3
          i32.add
          global.set 51
        else
          local.get 1
          local.get 2
          i32.add
          local.set 6
          i32.const 1
          local.get 6
          local.get 6
          i32.eqz
          select
          local.set 6
          local.get 4
          local.get 6
          i32.rem_u
          local.set 5
          local.get 5
          local.get 3
          i32.lt_u
          if ;; label = @4
            global.get 0
            local.get 5
            i32.sub
            global.set 51
          else
            global.get 52
            i32.const 1
            i32.xor
            global.set 52
            global.get 0
            local.get 5
            local.get 3
            i32.sub
            i32.sub
            global.set 51
          end
        end
      end
      global.get 52
      i32.eqz
      local.get 1
      i32.const 0
      i32.ne
      i32.and
    else
      i32.const 1
      global.set 53
      i32.const 0
    end
    global.set 19
    global.get 1
    local.set 7
    i32.const 1
    i32.const 16
    i32.const 128
    i32.const 2576
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 16
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 20
    global.get 2
    local.set 7
    i32.const 1
    i32.const 144
    i32.const 128
    i32.const 2580
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 144
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 21
    global.get 3
    local.set 7
    i32.const 1
    i32.const 272
    i32.const 128
    i32.const 2584
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 272
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 22
    global.get 4
    local.set 7
    i32.const 1
    i32.const 400
    i32.const 128
    i32.const 2588
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 400
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 23
    global.get 5
    local.set 7
    i32.const 1
    i32.const 528
    i32.const 128
    i32.const 2592
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 528
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 24
    global.get 6
    local.set 7
    i32.const 1
    i32.const 656
    i32.const 128
    i32.const 2600
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 656
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 25
    global.get 7
    local.set 7
    i32.const 1
    i32.const 784
    i32.const 128
    i32.const 2604
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 784
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 26
  )
  (func (;14;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 8
    local.set 7
    i32.const 1
    i32.const 912
    i32.const 128
    i32.const 2608
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 912
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 27
    global.get 9
    local.set 7
    i32.const 1
    i32.const 1040
    i32.const 128
    i32.const 2612
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1040
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 28
    global.get 10
    local.set 7
    i32.const 1
    i32.const 1168
    i32.const 128
    i32.const 2616
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1168
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 29
    global.get 11
    local.set 7
    i32.const 1
    i32.const 1296
    i32.const 128
    i32.const 2620
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1296
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 30
    global.get 12
    local.set 7
    i32.const 1
    i32.const 1424
    i32.const 128
    i32.const 2628
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1424
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 31
    global.get 13
    local.set 7
    i32.const 1
    i32.const 1552
    i32.const 128
    i32.const 2636
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1552
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 32
    global.get 17
    local.set 7
    i32.const 1
    i32.const 1680
    i32.const 128
    i32.const 2644
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1680
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 33
    global.get 18
    local.set 7
    i32.const 1
    i32.const 1808
    i32.const 128
    i32.const 2648
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1808
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 34
  )
  (func (;15;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32 f32 f32)
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 14
    i32.const 0
    i32.ne
    select
    global.set 35
    global.get 35
    local.set 7
    i32.const 1
    i32.const 1936
    i32.const 128
    i32.const 2652
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 1936
    i32.const 128
    local.get 7
    i32.reinterpret_f32
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 36
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 16
    global.get 20
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 144
    global.get 21
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 272
    global.get 22
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 400
    global.get 23
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 37
    global.get 19
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 56
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 56
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 2660
      i32.const 130
      i32.const 2064
      global.get 37
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 2660
      i32.const 130
      i32.const 2792
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 2660
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 38
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 528
    global.get 24
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 656
    global.get 25
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 784
    global.get 26
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 912
    global.get 27
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 39
    global.get 19
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 59
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 59
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 2796
      i32.const 130
      i32.const 2192
      global.get 39
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 2796
      i32.const 130
      i32.const 2792
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 2796
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 40
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 1040
    global.get 28
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 1168
    global.get 29
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 1296
    global.get 30
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 1424
    global.get 31
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 41
    global.get 19
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 62
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 62
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 2928
      i32.const 130
      i32.const 2320
      global.get 41
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 2928
      i32.const 130
      i32.const 2792
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 2928
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 42
  )
  (func (;16;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32 f32 f32)
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 1552
    global.get 32
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 1680
    global.get 33
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 1808
    global.get 34
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 2656
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 1936
    global.get 36
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 43
    global.get 19
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 65
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 65
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 3060
      i32.const 130
      i32.const 2448
      global.get 43
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 3060
      i32.const 130
      i32.const 2792
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 3060
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 44
  )
  (func (;17;) (type 13) (param i32 i32 i32 i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    local.get 4
    local.get 5
    local.get 6
    call 7
  )
  (func (;18;) (type 12) (param i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    call 6
  )
  (table (;0;) 2 2 funcref)
  (elem (;0;) (i32.const 0) func 17 18)
  (data (;0;) (i32.const 2576) "abs=")
  (data (;1;) (i32.const 2580) "min=")
  (data (;2;) (i32.const 2584) "max=")
  (data (;3;) (i32.const 2588) "lim=")
  (data (;4;) (i32.const 2592) "sqrt=")
  (data (;5;) (i32.const 2600) "sq=")
  (data (;6;) (i32.const 2604) "pow=")
  (data (;7;) (i32.const 2608) "mod=")
  (data (;8;) (i32.const 2612) "sw=")
  (data (;9;) (i32.const 2616) "mux=")
  (data (;10;) (i32.const 2620) "dms1=")
  (data (;11;) (i32.const 2628) "dms2=")
  (data (;12;) (i32.const 2636) "dms3=")
  (data (;13;) (i32.const 2644) "tt=")
  (data (;14;) (i32.const 2648) "deb=")
  (data (;15;) (i32.const 2652) "gen=")
  (data (;16;) (i32.const 2656) " ")
  (data (;17;) (i32.const 2792) "\0d\0a")
  (@custom "crab.mem" (after data) "\c0\0c\00\00")
)

