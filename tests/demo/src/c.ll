
; ModuleID = 'test_ir.ll'
source_filename = "test_ir.ll"

; 外部声明函数（无函数体，用于测试 is_declaration() == true）
declare i32 @external_decl(i32)

define i32 @target_func_a(i32 %x) {
entry:
  %call = call i32 @external_decl(i32 %x)
  %add = add nsw i32 %call, 42
  ret i32 %add
}

define i32 @target_func_b(i32 %y) {
entry:
  %mul = mul nsw i32 %y, 2
  ret i32 %mul
}