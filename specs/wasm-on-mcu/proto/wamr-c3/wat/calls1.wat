;; Micro benchmark: 100 host calls with 1 argument (like crab.tag_get_i) per step.
(module
  (@custom "crab.mem" "\00\00\00\00")
  (import "crab" "tag_get_i" (func $get (param i32) (result i32)))
  (global $acc (mut i32) (i32.const 0))
  (func (export "abi_version") (result i32) i32.const 3)
  (func (export "init"))
  (func (export "step") (param $dt i32) (local $i i32)
    (loop $l
      (global.set $acc (i32.add (global.get $acc) (call $get (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 100))))))
