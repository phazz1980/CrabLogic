;; Micro benchmark: the same loop without host calls (baseline for calls1/calls7).
(module
  (@custom "crab.mem" "\00\00\00\00")
  (global $acc (mut i32) (i32.const 0))
  (func (export "abi_version") (result i32) i32.const 3)
  (func (export "init"))
  (func (export "step") (param $dt i32) (local $i i32)
    (loop $l
      (global.set $acc (i32.add (global.get $acc) (i32.mul (local.get $i) (global.get $acc))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 100))))))
