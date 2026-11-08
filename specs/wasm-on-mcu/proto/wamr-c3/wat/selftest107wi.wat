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
  (global (;1;) (mut i32) i32.const 0)
  (global (;2;) (mut i32) i32.const 0)
  (global (;3;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;4;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;5;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;6;) (mut i32) i32.const 0)
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
  (global (;22;) (mut i32) i32.const 0)
  (global (;23;) (mut i32) i32.const 0)
  (global (;24;) (mut i32) i32.const 0)
  (global (;25;) (mut i32) i32.const 0)
  (global (;26;) (mut i32) i32.const 0)
  (global (;27;) (mut i32) i32.const 0)
  (global (;28;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;29;) (mut i32) i32.const 0)
  (global (;30;) (mut i32) i32.const 0)
  (global (;31;) (mut i32) i32.const 0)
  (global (;32;) (mut i32) i32.const 0)
  (global (;33;) (mut i32) i32.const 0)
  (global (;34;) (mut i32) i32.const 0)
  (global (;35;) (mut i32) i32.const 0)
  (global (;36;) (mut i32) i32.const 0)
  (global (;37;) (mut i32) i32.const 0)
  (global (;38;) (mut i32) i32.const 0)
  (global (;39;) (mut i32) i32.const 0)
  (global (;40;) (mut i32) i32.const 0)
  (global (;41;) (mut i32) i32.const 0)
  (global (;42;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;43;) (mut i32) i32.const 0)
  (global (;44;) (mut i32) i32.const 0)
  (global (;45;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;46;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;47;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;48;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;49;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;50;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;51;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;52;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;53;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;54;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;55;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;56;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;57;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;58;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;59;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;60;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;61;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;62;) (mut i32) i32.const 0)
  (global (;63;) (mut i32) i32.const 0)
  (global (;64;) (mut i32) i32.const 0)
  (global (;65;) (mut i32) i32.const 0)
  (global (;66;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;67;) (mut i32) i32.const 0)
  (global (;68;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;69;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;70;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;71;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;72;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;73;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;74;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;75;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;76;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;77;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;78;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;79;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;80;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;81;) (mut i32) i32.const 0)
  (global (;82;) (mut i32) i32.const 0)
  (global (;83;) (mut i32) i32.const 0)
  (global (;84;) (mut i32) i32.const 0)
  (global (;85;) (mut i32) i32.const 0)
  (global (;86;) (mut i32) i32.const 0)
  (global (;87;) (mut i32) i32.const 0)
  (global (;88;) (mut i32) i32.const 0)
  (global (;89;) (mut i32) i32.const 0)
  (global (;90;) (mut i32) i32.const 0)
  (global (;91;) (mut i32) i32.const 0)
  (global (;92;) (mut i32) i32.const 0)
  (global (;93;) (mut i32) i32.const 0)
  (global (;94;) (mut i32) i32.const 0)
  (global (;95;) (mut i32) i32.const 0)
  (global (;96;) (mut i32) i32.const 0)
  (global (;97;) (mut i32) i32.const 0)
  (global (;98;) (mut i32) i32.const 0)
  (global (;99;) (mut i32) i32.const 0)
  (global (;100;) (mut i32) i32.const 0)
  (global (;101;) (mut i32) i32.const 0)
  (global (;102;) (mut i32) i32.const 0)
  (global (;103;) (mut i32) i32.const 0)
  (global (;104;) (mut i32) i32.const 0)
  (global (;105;) (mut i32) i32.const 0)
  (global (;106;) (mut i32) i32.const 0)
  (global (;107;) (mut i32) i32.const 0)
  (global (;108;) (mut i32) i32.const 0)
  (global (;109;) (mut i32) i32.const 0)
  (global (;110;) (mut i32) i32.const 0)
  (global (;111;) (mut i32) i32.const 0)
  (global (;112;) (mut i32) i32.const 0)
  (global (;113;) (mut i32) i32.const 0)
  (global (;114;) (mut i32) i32.const 0)
  (global (;115;) (mut i32) i32.const 0)
  (global (;116;) (mut i32) i32.const 0)
  (global (;117;) (mut i32) i32.const 0)
  (global (;118;) (mut i32) i32.const 0)
  (global (;119;) (mut i32) i32.const 0)
  (global (;120;) (mut i32) i32.const 0)
  (global (;121;) (mut i32) i32.const 0)
  (global (;122;) (mut i32) i32.const 0)
  (global (;123;) (mut i32) i32.const 0)
  (global (;124;) (mut i32) i32.const 0)
  (global (;125;) (mut i32) i32.const 0)
  (global (;126;) (mut i32) i32.const 0)
  (global (;127;) (mut i32) i32.const 0)
  (global (;128;) (mut i32) i32.const 0)
  (global (;129;) (mut i32) i32.const 0)
  (global (;130;) (mut i32) i32.const 0)
  (global (;131;) (mut i32) i32.const 0)
  (global (;132;) (mut i32) i32.const 0)
  (global (;133;) (mut i32) i32.const 0)
  (global (;134;) (mut i32) i32.const 0)
  (global (;135;) (mut i32) i32.const 0)
  (global (;136;) (mut i32) i32.const 0)
  (global (;137;) (mut i32) i32.const 0)
  (global (;138;) (mut i32) i32.const 0)
  (global (;139;) (mut i32) i32.const 0)
  (global (;140;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;141;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;142;) (mut i32) i32.const 0)
  (global (;143;) (mut i32) i32.const 0)
  (global (;144;) (mut f32) f32.const 0x0p+0 (;=0;))
  (global (;145;) (mut i32) i32.const 0)
  (global (;146;) (mut i32) i32.const 0)
  (global (;147;) (mut i32) i32.const 0)
  (global (;148;) (mut i32) i32.const 0)
  (global (;149;) (mut i32) i32.const 0)
  (global (;150;) (mut i32) i32.const 0)
  (global (;151;) (mut i32) i32.const 0)
  (global (;152;) (mut i32) i32.const 0)
  (global (;153;) (mut i32) i32.const -1831433054)
  (global (;154;) (mut i32) i32.const 0)
  (global (;155;) (mut i32) i32.const 0)
  (global (;156;) (mut i32) i32.const 0)
  (global (;157;) (mut i32) i32.const 0)
  (global (;158;) (mut i32) i32.const 0)
  (global (;159;) (mut i32) i32.const 0)
  (global (;160;) (mut i32) i32.const 0)
  (global (;161;) (mut i32) i32.const 0)
  (global (;162;) (mut i32) i32.const 0)
  (global (;163;) (mut i32) i32.const 0)
  (global (;164;) (mut i32) i32.const 0)
  (global (;165;) (mut i32) i32.const 0)
  (global (;166;) (mut i32) i32.const 0)
  (global (;167;) (mut i32) i32.const 0)
  (global (;168;) (mut i32) i32.const 0)
  (global (;169;) (mut i32) i32.const 0)
  (global (;170;) (mut i32) i32.const 0)
  (global (;171;) (mut i32) i32.const 0)
  (global (;172;) (mut i32) i32.const 0)
  (global (;173;) (mut i32) i32.const 0)
  (global (;174;) (mut i32) i32.const 0)
  (global (;175;) (mut i32) i32.const 0)
  (global (;176;) (mut i32) i32.const 0)
  (global (;177;) (mut i32) i32.const 0)
  (global (;178;) (mut i32) i32.const 0)
  (global (;179;) (mut i32) i32.const 0)
  (global (;180;) (mut i32) i32.const 0)
  (global (;181;) (mut i32) i32.const 0)
  (global (;182;) (mut i32) i32.const 0)
  (global (;183;) (mut i32) i32.const 0)
  (global (;184;) (mut i32) i32.const 0)
  (global (;185;) (mut i32) i32.const 0)
  (global (;186;) (mut i32) i32.const 0)
  (global (;187;) (mut i32) i32.const 0)
  (export "memory" (memory 0))
  (export "abi_version" (func 9))
  (export "step" (func 10))
  (export "init" (func 11))
  (func (;9;) (type 4) (result i32)
    i32.const 3
  )
  (func (;10;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 0
    local.get 0
    i32.add
    global.set 0
    f32.const 0x1.4p+2 (;=5;)
    f32.const 0x1.8p+1 (;=3;)
    f32.gt
    global.set 1
    i32.const 1
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      i32.const 1
    else
      i32.const 1
      i32.const 0
      i32.ne
      if (result i32) ;; label = @2
        i32.const 0
      else
        global.get 2
      end
    end
    global.set 2
    f32.const 0x1.ep+4 (;=30;)
    local.set 7
    i32.const 0
    i32.eqz
    if (result f32) ;; label = @1
      local.get 7
      f32.const 0x1.1df46ap-6 (;=0.017453292;)
      f32.mul
    else
      local.get 7
    end
    local.set 7
    i32.const 0
    local.get 7
    local.get 7
    call 8
    global.set 3
    f32.const 0x1.ep+5 (;=60;)
    local.set 7
    i32.const 0
    i32.eqz
    if (result f32) ;; label = @1
      local.get 7
      f32.const 0x1.1df46ap-6 (;=0.017453292;)
      f32.mul
    else
      local.get 7
    end
    local.set 7
    i32.const 1
    local.get 7
    local.get 7
    call 8
    global.set 4
    f32.const 0x1.68p+5 (;=45;)
    local.set 7
    i32.const 0
    i32.eqz
    if (result f32) ;; label = @1
      local.get 7
      f32.const 0x1.1df46ap-6 (;=0.017453292;)
      f32.mul
    else
      local.get 7
    end
    local.set 7
    i32.const 2
    local.get 7
    local.get 7
    call 8
    global.set 5
    i32.const 12
    i32.const 10
    i32.and
    global.set 6
    i32.const 12
    i32.const 10
    i32.or
    global.set 7
    i32.const 12
    i32.const 10
    i32.xor
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
    local.get 0
    call 17
    local.get 0
    call 18
    local.get 0
    call 19
    local.get 0
    call 20
    local.get 0
    call 21
    local.get 0
    call 22
    local.get 0
    call 23
    local.get 0
    call 24
  )
  (func (;11;) (type 6)
    (local i32)
  )
  (func (;12;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    i32.const 0
    i32.const -1
    i32.xor
    global.set 9
    i32.const 5
    i32.const 2
    i32.shr_s
    i32.const 1
    i32.and
    global.set 10
    i32.const 1
    i32.const 3
    i32.shl
    local.set 2
    i32.const 0
    local.set 1
    i32.const 1
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      local.get 1
      local.get 2
      i32.or
    else
      local.get 1
      local.get 2
      i32.const -1
      i32.xor
      i32.and
    end
    global.set 11
    i32.const 5
    local.set 1
    local.get 1
    i32.const 0
    i32.shr_s
    i32.const 1
    i32.and
    global.set 12
    local.get 1
    i32.const 1
    i32.shr_s
    i32.const 1
    i32.and
    global.set 13
    local.get 1
    i32.const 2
    i32.shr_s
    i32.const 1
    i32.and
    global.set 14
    local.get 1
    i32.const 3
    i32.shr_s
    i32.const 1
    i32.and
    global.set 15
    local.get 1
    i32.const 4
    i32.shr_s
    i32.const 1
    i32.and
    global.set 16
    local.get 1
    i32.const 5
    i32.shr_s
    i32.const 1
    i32.and
    global.set 17
    local.get 1
    i32.const 6
    i32.shr_s
    i32.const 1
    i32.and
    global.set 18
    local.get 1
    i32.const 7
    i32.shr_s
    i32.const 1
    i32.and
    global.set 19
    i32.const 0
    i32.const 1
    i32.const 0
    i32.ne
    i32.const 1
    i32.mul
    i32.or
    i32.const 0
    i32.const 0
    i32.ne
    i32.const 2
    i32.mul
    i32.or
    i32.const 1
    i32.const 0
    i32.ne
    i32.const 4
    i32.mul
    i32.or
    i32.const 0
    i32.const 0
    i32.ne
    i32.const 8
    i32.mul
    i32.or
    i32.const 0
    i32.const 0
    i32.ne
    i32.const 16
    i32.mul
    i32.or
    i32.const 0
    i32.const 0
    i32.ne
    i32.const 32
    i32.mul
    i32.or
    i32.const 0
    i32.const 0
    i32.ne
    i32.const 64
    i32.mul
    i32.or
    i32.const 1
    i32.const 0
    i32.ne
    i32.const 128
    i32.mul
    i32.or
    global.set 20
    i32.const 2
    local.set 1
    local.get 1
    i32.const 0
    i32.eq
    global.set 21
    local.get 1
    i32.const 1
    i32.eq
    global.set 22
    local.get 1
    i32.const 2
    i32.eq
    global.set 23
    local.get 1
    i32.const 3
    i32.eq
    global.set 24
    i32.const 0
    local.set 1
    i32.const 3
    local.get 1
    i32.const 1
    i32.const 0
    i32.ne
    select
    local.set 1
    i32.const 2
    local.get 1
    i32.const 0
    i32.const 0
    i32.ne
    select
    local.set 1
    i32.const 1
    local.get 1
    i32.const 1
    i32.const 0
    i32.ne
    select
    local.set 1
    i32.const 0
    local.get 1
    i32.const 0
    i32.const 0
    i32.ne
    select
    global.set 26
    i32.const 0
    i32.const 0
    i32.const 0
    i32.ne
    i32.or
    i32.const 1
    i32.const 0
    i32.ne
    i32.or
    i32.const 0
    i32.const 0
    i32.ne
    i32.or
    i32.const 1
    i32.const 0
    i32.ne
    i32.or
    global.set 25
    f32.const 0x1.4p+1 (;=2.5;)
    local.set 7
    local.get 7
    f64.promote_f32
    f64.const 0x1p-1 (;=0.5;)
    local.get 7
    f64.promote_f32
    f64.copysign
    f64.add
    f64.trunc
    i32.trunc_sat_f64_s
    global.set 27
  )
  (func (;13;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    i32.const 7
    f32.convert_i32_s
    global.set 28
    i32.const 4660
    local.set 1
    local.get 1
    i32.const 0
    i32.shr_s
    i32.const 255
    i32.and
    global.set 30
    local.get 1
    i32.const 8
    i32.shr_s
    i32.const 255
    i32.and
    global.set 29
    i32.const 18
    i32.const 255
    i32.and
    i32.const 8
    i32.shl
    i32.const 52
    i32.const 255
    i32.and
    i32.or
    global.set 31
    i32.const 16909060
    local.set 1
    local.get 1
    i32.const 0
    i32.shr_u
    i32.const 255
    i32.and
    global.set 32
    local.get 1
    i32.const 8
    i32.shr_u
    i32.const 255
    i32.and
    global.set 33
    local.get 1
    i32.const 16
    i32.shr_u
    i32.const 255
    i32.and
    global.set 34
    local.get 1
    i32.const 24
    i32.shr_u
    i32.const 255
    i32.and
    global.set 35
    i32.const 0
    i32.const 4
    i32.const 255
    i32.and
    i32.const 0
    i32.shl
    i32.or
    i32.const 3
    i32.const 255
    i32.and
    i32.const 8
    i32.shl
    i32.or
    i32.const 2
    i32.const 255
    i32.and
    i32.const 16
    i32.shl
    i32.or
    i32.const 1
    i32.const 255
    i32.and
    i32.const 24
    i32.shl
    i32.or
    global.set 36
    i32.const 8
    i32.const 0
    i32.const 0
    i32.const 6544
    i32.const 11
    i32.const 6556
    i32.const 3
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 38
    local.get 1
    i32.const 0
    i32.ge_s
    global.set 37
    i32.const 1
    local.set 1
    i32.const 9
    i32.const 0
    i32.const 0
    i32.const 6560
    i32.const 3
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 2
    local.get 2
    i32.const 0
    i32.ge_s
    if ;; label = @1
      i32.const 7
      i32.const 16
      i32.const 128
      i32.const 6560
      i32.const 3
      local.get 1
      i32.const 1
      i32.const 0
    call_indirect (type 13)
      global.set 39
    else
      i32.const 0
      local.set 2
      i32.const 0
      global.set 39
    end
    local.get 2
    global.set 40
    i32.const 99
    local.set 1
    i32.const 1
    i32.const 144
    i32.const 128
    i32.const 6564
    i32.const 2
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 2
    i32.const 4
    i32.const 144
    i32.const 128
    local.get 1
    i32.const 0
    local.get 2
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    global.set 41
  )
  (func (;14;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    f32.const 0x1p+1 (;=2;)
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.set 8
    f32.const 0x0p+0 (;=0;)
    local.set 9
    f32.const 0x0p+0 (;=0;)
    local.set 10
    f32.const 0x1.9p+6 (;=100;)
    local.set 11
    i32.const 1
    i32.const 0
    i32.ne
    if ;; label = @1
      f32.const 0x1.4p+3 (;=10;)
      f32.const 0x0p+0 (;=0;)
      f32.sub
      local.set 12
      global.get 143
      if ;; label = @2
        global.get 0
        global.get 142
        i32.sub
        f32.convert_i32_u
        f32.const 0x1.f4p+9 (;=1000;)
        f32.div
        local.set 13
        local.get 13
        f32.const 0x0p+0 (;=0;)
        f32.gt
        if ;; label = @3
          global.get 140
          local.get 12
          local.get 13
          f32.mul
          local.get 8
          f32.mul
          f32.add
          local.set 14
          local.get 14
          local.get 10
          f32.lt
          if (result f32) ;; label = @4
            local.get 10
          else
            local.get 14
            local.get 11
            f32.gt
            if (result f32) ;; label = @5
              local.get 11
            else
              local.get 14
            end
          end
          global.set 140
          local.get 7
          local.get 12
          f32.mul
          global.get 140
          f32.add
          local.get 9
          local.get 12
          global.get 141
          f32.sub
          local.get 13
          f32.div
          f32.mul
          f32.add
          local.set 14
          local.get 14
          local.get 10
          f32.lt
          if (result f32) ;; label = @4
            local.get 10
          else
            local.get 14
            local.get 11
            f32.gt
            if (result f32) ;; label = @5
              local.get 11
            else
              local.get 14
            end
          end
          global.set 42
          local.get 12
          global.set 141
          global.get 0
          global.set 142
        end
      else
        i32.const 1
        global.set 143
        global.get 0
        global.set 142
        local.get 12
        global.set 141
        local.get 7
        local.get 12
        f32.mul
        local.set 14
        local.get 14
        local.get 10
        f32.lt
        if (result f32) ;; label = @3
          local.get 10
        else
          local.get 14
          local.get 11
          f32.gt
          if (result f32) ;; label = @4
            local.get 11
          else
            local.get 14
          end
        end
        global.set 42
      end
    else
      i32.const 0
      global.set 143
      f32.const 0x0p+0 (;=0;)
      global.set 140
      local.get 10
      global.set 42
    end
    i32.const 1
    i32.const 0
    i32.ne
    i32.const 1
    i32.const 0
    i32.ne
    i32.and
    i32.const 1
    i32.const 0
    i32.ne
    i32.and
    global.set 43
    i32.const 1
    i32.const 0
    i32.ne
    i32.const 1
    i32.const 0
    i32.ne
    i32.xor
    i32.const 1
    i32.const 0
    i32.ne
    i32.xor
    global.set 44
    f32.const 0x1.ep+3 (;=15;)
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
    f32.const 0x1.8cp+6 (;=99;)
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 15
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 14
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 13
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 12
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 11
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 10
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 9
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 8
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 7
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 6
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 5
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 4
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 3
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 2
    i32.lt_s
    select
    local.set 7
    f32.const 0x0p+0 (;=0;)
    local.get 7
    local.get 1
    i32.const 1
    i32.lt_s
    select
    local.set 7
    local.get 7
    global.set 45
    f32.const 0x1.ep+3 (;=15;)
    i32.trunc_sat_f32_s
    local.set 1
    f32.const 0x1.4p+2 (;=5;)
    local.set 7
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 0
    i32.eq
    select
    global.set 46
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 1
    i32.eq
    select
    global.set 54
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 2
    i32.eq
    select
    global.set 55
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 3
    i32.eq
    select
    global.set 56
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 4
    i32.eq
    select
    global.set 57
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 5
    i32.eq
    select
    global.set 58
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 6
    i32.eq
    select
    global.set 59
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 7
    i32.eq
    select
    global.set 60
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 8
    i32.eq
    select
    global.set 61
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 9
    i32.eq
    select
    global.set 47
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 10
    i32.eq
    select
    global.set 48
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 11
    i32.eq
    select
    global.set 49
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 12
    i32.eq
    select
    global.set 50
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 13
    i32.eq
    select
    global.set 51
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 14
    i32.eq
    select
    global.set 52
    local.get 7
    f32.const 0x0p+0 (;=0;)
    local.get 1
    i32.const 15
    i32.eq
    select
    global.set 53
    f32.const 0x0p+0 (;=0;)
    local.set 8
    f32.const 0x1p+0 (;=1;)
    local.set 7
    global.get 145
    local.get 7
    global.get 144
    f32.sub
    f32.abs
    local.get 8
    f32.gt
    i32.and
    local.set 1
    global.get 145
    i32.eqz
    local.get 1
    i32.or
    if ;; label = @1
      local.get 7
      global.set 144
    end
    i32.const 1
    global.set 145
    local.get 1
    global.set 62
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 6568
    i32.const 1
    i32.const 6572
    global.get 146
    i32.const 0
    call_indirect (type 13)
    local.set 1
    global.get 147
    local.get 1
    i32.eqz
    i32.and
    global.set 63
    i32.const 1
    i32.const 6572
    i32.const 128
    i32.const 6568
    i32.const 1
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    global.set 146
    i32.const 1
    global.set 147
    i32.const 2
    i32.const 1
    i32.shr_s
    i32.const 1
    i32.and
    local.set 1
    global.get 149
    local.get 1
    global.get 148
    i32.ne
    i32.and
    local.get 1
    global.set 148
    i32.const 1
    global.set 149
    global.set 64
  )
  (func (;15;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    i32.const 1
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      i32.const 300
      local.set 1
      i32.const 300
      local.set 2
      global.get 152
      if ;; label = @2
        i32.const 0
        global.set 152
        i32.const 0
        global.set 151
        global.get 0
        global.set 150
      end
      local.get 2
      local.get 1
      global.get 151
      select
      local.set 3
      global.get 0
      global.get 150
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
          global.get 151
          i32.const 1
          i32.xor
          global.set 151
          global.get 150
          local.get 3
          i32.add
          global.set 150
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
            global.set 150
          else
            global.get 151
            i32.const 1
            i32.xor
            global.set 151
            global.get 0
            local.get 5
            local.get 3
            i32.sub
            i32.sub
            global.set 150
          end
        end
      end
      global.get 151
      i32.eqz
      local.get 1
      i32.const 0
      i32.ne
      i32.and
    else
      i32.const 1
      global.set 152
      i32.const 0
    end
    global.set 65
    f32.const 0x1.4p+3 (;=10;)
    local.set 7
    f32.const 0x1.4p+4 (;=20;)
    local.set 8
    global.get 65
    i32.const 0
    i32.ne
    local.set 1
    local.get 1
    global.get 154
    i32.eqz
    i32.and
    if ;; label = @1
      global.get 153
      global.get 153
      i32.const 13
      i32.shl
      i32.xor
      global.set 153
      global.get 153
      global.get 153
      i32.const 17
      i32.shr_u
      i32.xor
      global.set 153
      global.get 153
      global.get 153
      i32.const 5
      i32.shl
      i32.xor
      global.set 153
      local.get 7
      local.get 8
      local.get 7
      f32.sub
      global.get 153
      i32.const 10001
      i32.rem_u
      f32.convert_i32_u
      f32.const 0x1.388p+13 (;=10000;)
      f32.div
      f32.mul
      f32.add
      global.set 66
    end
    local.get 1
    global.set 154
    i32.const 1
    i32.const 0
    i32.ne
    if (result i32) ;; label = @1
      i32.const 100
      local.set 1
      i32.const 900
      local.set 2
      global.get 157
      if ;; label = @2
        i32.const 0
        global.set 157
        i32.const 0
        global.set 156
        global.get 0
        global.set 155
      end
      local.get 2
      local.get 1
      global.get 156
      select
      local.set 3
      global.get 0
      global.get 155
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
          global.get 156
          i32.const 1
          i32.xor
          global.set 156
          global.get 155
          local.get 3
          i32.add
          global.set 155
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
            global.set 155
          else
            global.get 156
            i32.const 1
            i32.xor
            global.set 156
            global.get 0
            local.get 5
            local.get 3
            i32.sub
            i32.sub
            global.set 155
          end
        end
      end
      global.get 156
      i32.eqz
      local.get 1
      i32.const 0
      i32.ne
      i32.and
    else
      i32.const 1
      global.set 157
      i32.const 0
    end
    global.set 67
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 1
    i32.const 0
    i32.ne
    select
    global.set 68
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 2
    i32.const 0
    i32.ne
    select
    global.set 69
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 10
    i32.const 0
    i32.ne
    select
    global.set 70
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 12
    i32.const 0
    i32.ne
    select
    global.set 71
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 13
    i32.const 0
    i32.ne
    select
    global.set 72
  )
  (func (;16;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 14
    i32.const 0
    i32.ne
    select
    global.set 73
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 23
    i32.const 0
    i32.ne
    select
    global.set 74
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 25
    i32.const 0
    i32.ne
    select
    global.set 75
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 43
    i32.const 0
    i32.ne
    select
    global.set 76
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 44
    i32.const 0
    i32.ne
    select
    global.set 77
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 62
    i32.const 0
    i32.ne
    select
    global.set 78
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 63
    i32.const 0
    i32.ne
    select
    global.set 79
    f32.const 0x1p+0 (;=1;)
    f32.const 0x0p+0 (;=0;)
    global.get 64
    i32.const 0
    i32.ne
    select
    global.set 80
  )
  (func (;17;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 68
    local.set 7
    i32.const 1
    i32.const 272
    i32.const 128
    i32.const 6700
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
    global.set 81
    global.get 69
    local.set 7
    i32.const 1
    i32.const 400
    i32.const 128
    i32.const 6704
    i32.const 3
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
    global.set 82
    global.get 3
    local.set 7
    i32.const 1
    i32.const 528
    i32.const 128
    i32.const 6708
    i32.const 4
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
    global.set 83
    global.get 4
    local.set 7
    i32.const 1
    i32.const 656
    i32.const 128
    i32.const 6712
    i32.const 4
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
    global.set 84
    global.get 5
    local.set 7
    i32.const 1
    i32.const 784
    i32.const 128
    i32.const 6716
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
    global.set 85
    global.get 6
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 912
    i32.const 128
    i32.const 6720
    i32.const 5
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
    global.set 86
    global.get 7
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 1040
    i32.const 128
    i32.const 6728
    i32.const 4
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
    global.set 87
    global.get 8
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 1168
    i32.const 128
    i32.const 6732
    i32.const 5
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
    global.set 88
  )
  (func (;18;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 9
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 1296
    i32.const 128
    i32.const 6740
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
    global.set 89
    global.get 70
    local.set 7
    i32.const 1
    i32.const 1424
    i32.const 128
    i32.const 6748
    i32.const 6
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
    global.set 90
    global.get 11
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 1552
    i32.const 128
    i32.const 6756
    i32.const 7
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
    global.set 91
    global.get 71
    local.set 7
    i32.const 1
    i32.const 1680
    i32.const 128
    i32.const 6764
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
    global.set 92
    global.get 72
    local.set 7
    i32.const 1
    i32.const 1808
    i32.const 128
    i32.const 6768
    i32.const 3
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
    global.set 93
    global.get 73
    local.set 7
    i32.const 1
    i32.const 1936
    i32.const 128
    i32.const 6772
    i32.const 3
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
    global.set 94
    global.get 20
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 2064
    i32.const 128
    i32.const 6776
    i32.const 7
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2064
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
    global.set 95
    global.get 74
    local.set 7
    i32.const 1
    i32.const 2192
    i32.const 128
    i32.const 6784
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2192
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
    global.set 96
  )
  (func (;19;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 26
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 2320
    i32.const 128
    i32.const 6788
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2320
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
    global.set 97
    global.get 75
    local.set 7
    i32.const 1
    i32.const 2448
    i32.const 128
    i32.const 6792
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2448
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
    global.set 98
    global.get 27
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 2576
    i32.const 128
    i32.const 6796
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2576
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
    global.set 99
    global.get 28
    local.set 7
    i32.const 1
    i32.const 2704
    i32.const 128
    i32.const 6800
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2704
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
    global.set 100
    global.get 30
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 2832
    i32.const 128
    i32.const 6804
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2832
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
    global.set 101
    global.get 29
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 2960
    i32.const 128
    i32.const 6808
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 2960
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
    global.set 102
    global.get 31
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3088
    i32.const 128
    i32.const 6812
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3088
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
    global.set 103
    global.get 32
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3216
    i32.const 128
    i32.const 6820
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3216
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
    global.set 104
  )
  (func (;20;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 33
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3344
    i32.const 128
    i32.const 6824
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3344
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
    global.set 105
    global.get 34
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3472
    i32.const 128
    i32.const 6828
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3472
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
    global.set 106
    global.get 35
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3600
    i32.const 128
    i32.const 6832
    i32.const 3
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3600
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
    global.set 107
    global.get 36
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3728
    i32.const 128
    i32.const 6836
    i32.const 6
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3728
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
    global.set 108
    global.get 38
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3856
    i32.const 128
    i32.const 6844
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3856
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
    global.set 109
    global.get 40
    f32.convert_i32_s
    local.set 7
    i32.const 1
    i32.const 3984
    i32.const 128
    i32.const 6852
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 3984
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
    global.set 110
    global.get 76
    local.set 7
    i32.const 1
    i32.const 4112
    i32.const 128
    i32.const 6860
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4112
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
    global.set 111
    global.get 77
    local.set 7
    i32.const 1
    i32.const 4240
    i32.const 128
    i32.const 6868
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4240
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
    global.set 112
  )
  (func (;21;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 42
    local.set 7
    i32.const 1
    i32.const 4368
    i32.const 128
    i32.const 6876
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4368
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
    global.set 113
    global.get 45
    local.set 7
    i32.const 1
    i32.const 4496
    i32.const 128
    i32.const 6880
    i32.const 6
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4496
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
    global.set 114
    global.get 53
    local.set 7
    i32.const 1
    i32.const 4624
    i32.const 128
    i32.const 6888
    i32.const 6
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4624
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
    global.set 115
    global.get 78
    local.set 7
    i32.const 1
    i32.const 4752
    i32.const 128
    i32.const 6896
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4752
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
    global.set 116
    global.get 79
    local.set 7
    i32.const 1
    i32.const 4880
    i32.const 128
    i32.const 6900
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 4880
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
    global.set 117
    global.get 80
    local.set 7
    i32.const 1
    i32.const 5008
    i32.const 128
    i32.const 6908
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 5008
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
    global.set 118
    global.get 66
    local.set 7
    i32.const 1
    i32.const 5136
    i32.const 128
    i32.const 6916
    i32.const 4
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 2
    i32.const 5136
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
    global.set 119
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 272
    global.get 81
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 400
    global.get 82
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 528
    global.get 83
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5264
    i32.const 128
    i32.const 656
    global.get 84
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 120
  )
  (func (;22;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 160
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 160
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 6924
      i32.const 130
      i32.const 5264
      global.get 120
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 6924
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 6924
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 121
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 784
    global.get 85
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 912
    global.get 86
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 1040
    global.get 87
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5392
    i32.const 128
    i32.const 1168
    global.get 88
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 122
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 163
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 163
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7060
      i32.const 130
      i32.const 5392
      global.get 122
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7060
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7060
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 123
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 1296
    global.get 89
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 1424
    global.get 90
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 1552
    global.get 91
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5520
    i32.const 128
    i32.const 1680
    global.get 92
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 124
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 166
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 166
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7192
      i32.const 130
      i32.const 5520
      global.get 124
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7192
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7192
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 125
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 1808
    global.get 93
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 1936
    global.get 94
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 2064
    global.get 95
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5648
    i32.const 128
    i32.const 2192
    global.get 96
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 126
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 169
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 169
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7324
      i32.const 130
      i32.const 5648
      global.get 126
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7324
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7324
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 127
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 2320
    global.get 97
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 2448
    global.get 98
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 2576
    global.get 99
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5776
    i32.const 128
    i32.const 2704
    global.get 100
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 128
  )
  (func (;23;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 172
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 172
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7456
      i32.const 130
      i32.const 5776
      global.get 128
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7456
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7456
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 129
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 2832
    global.get 101
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 2960
    global.get 102
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 3088
    global.get 103
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 5904
    i32.const 128
    i32.const 3216
    global.get 104
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 130
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 175
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 175
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7588
      i32.const 130
      i32.const 5904
      global.get 130
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7588
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7588
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 131
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 3344
    global.get 105
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 3472
    global.get 106
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 3600
    global.get 107
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6032
    i32.const 128
    i32.const 3728
    global.get 108
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 132
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 178
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 178
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7720
      i32.const 130
      i32.const 6032
      global.get 132
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7720
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7720
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 133
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 3856
    global.get 109
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 3984
    global.get 110
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 4112
    global.get 111
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6160
    i32.const 128
    i32.const 4240
    global.get 112
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 134
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 181
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 181
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7852
      i32.const 130
      i32.const 6160
      global.get 134
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7852
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7852
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 135
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 4368
    global.get 113
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 4496
    global.get 114
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 4624
    global.get 115
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6288
    i32.const 128
    i32.const 4752
    global.get 116
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 136
  )
  (func (;24;) (type 5) (param i32)
    (local i32 i32 i32 i32 i32 i32 f32 f32 f32 f32 f32 f32 f32 f32)
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 184
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 184
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 7984
      i32.const 130
      i32.const 6288
      global.get 136
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 7984
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 7984
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 137
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 4880
    global.get 117
    i32.const 0
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 5008
    global.get 118
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 5136
    global.get 119
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 6920
    i32.const 1
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    i32.const 1
    i32.const 6416
    i32.const 128
    i32.const 0
    i32.const 0
    local.get 1
    i32.const 0
    i32.const 0
    call_indirect (type 13)
    local.set 1
    local.get 1
    global.set 138
    global.get 67
    i32.const 0
    i32.ne
    local.set 2
    local.get 2
    global.get 187
    i32.eqz
    i32.and
    local.set 1
    local.get 2
    global.set 187
    i32.const 0
    local.set 3
    local.get 1
    if ;; label = @1
      i32.const 1
      i32.const 8116
      i32.const 130
      i32.const 6416
      global.get 138
      i32.const 0
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 1
      i32.const 8116
      i32.const 130
      i32.const 7056
      i32.const 2
      local.get 4
      i32.const 0
      i32.const 0
    call_indirect (type 13)
      local.set 4
      i32.const 6
      i32.const 0
      i32.const 8116
      local.get 4
      i32.const 1
    call_indirect (type 12)
      local.set 3
    end
    local.get 3
    global.set 139
  )
  (func (;25;) (type 13) (param i32 i32 i32 i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    local.get 4
    local.get 5
    local.get 6
    call 7
  )
  (func (;26;) (type 12) (param i32 i32 i32 i32) (result i32)
    local.get 0
    local.get 1
    local.get 2
    local.get 3
    call 6
  )
  (table (;0;) 2 2 funcref)
  (elem (;0;) (i32.const 0) func 25 26)
  (data (;0;) (i32.const 6544) "hello world")
  (data (;1;) (i32.const 6556) "wor")
  (data (;2;) (i32.const 6560) "abc")
  (data (;3;) (i32.const 6564) "ab")
  (data (;4;) (i32.const 6568) "x")
  (data (;5;) (i32.const 6700) "cmp=")
  (data (;6;) (i32.const 6704) "sr=")
  (data (;7;) (i32.const 6708) "sin=")
  (data (;8;) (i32.const 6712) "cos=")
  (data (;9;) (i32.const 6716) "tan=")
  (data (;10;) (i32.const 6720) "band=")
  (data (;11;) (i32.const 6728) "bor=")
  (data (;12;) (i32.const 6732) "bxor=")
  (data (;13;) (i32.const 6740) "bnot=")
  (data (;14;) (i32.const 6748) "bread=")
  (data (;15;) (i32.const 6756) "bwrite=")
  (data (;16;) (i32.const 6764) "b0=")
  (data (;17;) (i32.const 6768) "b1=")
  (data (;18;) (i32.const 6772) "b2=")
  (data (;19;) (i32.const 6776) "tobyte=")
  (data (;20;) (i32.const 6784) "dec=")
  (data (;21;) (i32.const 6788) "enc=")
  (data (;22;) (i32.const 6792) "any=")
  (data (;23;) (i32.const 6796) "f2i=")
  (data (;24;) (i32.const 6800) "i2f=")
  (data (;25;) (i32.const 6804) "lo=")
  (data (;26;) (i32.const 6808) "hi=")
  (data (;27;) (i32.const 6812) "word=")
  (data (;28;) (i32.const 6820) "d0=")
  (data (;29;) (i32.const 6824) "d1=")
  (data (;30;) (i32.const 6828) "d2=")
  (data (;31;) (i32.const 6832) "d3=")
  (data (;32;) (i32.const 6836) "dword=")
  (data (;33;) (i32.const 6844) "find=")
  (data (;34;) (i32.const 6852) "code=")
  (data (;35;) (i32.const 6860) "and3=")
  (data (;36;) (i32.const 6868) "xor3=")
  (data (;37;) (i32.const 6876) "pid=")
  (data (;38;) (i32.const 6880) "mux16=")
  (data (;39;) (i32.const 6888) "dms16=")
  (data (;40;) (i32.const 6896) "chg=")
  (data (;41;) (i32.const 6900) "schg=")
  (data (;42;) (i32.const 6908) "bchg=")
  (data (;43;) (i32.const 6916) "rnd=")
  (data (;44;) (i32.const 6920) " ")
  (data (;45;) (i32.const 7056) "\0d\0a")
  (@custom "crab.mem" (after data) "\80 \00\00")
)

