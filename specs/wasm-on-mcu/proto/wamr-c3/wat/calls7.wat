;; Micro benchmark: 100 host calls with 7 arguments (like crab.text) per step.
(module
  (@custom "crab.mem" "\00\00\01\00")
  (import "crab" "text" (func $text (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (memory 1)
  (global $acc (mut i32) (i32.const 0))
  (func (export "abi_version") (result i32) i32.const 3)
  (func (export "init"))
  (func (export "step") (param $dt i32) (local $i i32)
    (loop $l
      (global.set $acc (i32.add (global.get $acc)
        (call $text (i32.const 1) (i32.const 64) (i32.const 32) (local.get $i) (i32.const 2) (global.get $acc) (i32.const 0))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 100))))))
