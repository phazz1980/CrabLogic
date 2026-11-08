;; Test module: step never returns (the master watchdog must restart it)
(module
  ;; No memory: the master needs this section to start the module (`specs/mcu-memory`).
  (@custom "crab.mem" "\00\00\00\00")
  (func (export "abi_version") (result i32) i32.const 3)
  (func (export "step") (param i32) (loop $l (br $l))))
