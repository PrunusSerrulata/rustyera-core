use super::*;
use erabasic_vm::VmValue;

#[test]
fn emuera_color_arguments_accept_packed_or_three_channels() {
    assert_eq!(
        color_argument_value(&[VmValue::Integer(0x01_18_3c)]),
        Ok(0x01_18_3c)
    );
    assert_eq!(
        color_argument_value(&[
            VmValue::Integer(1),
            VmValue::Integer(24),
            VmValue::Integer(60),
        ]),
        Ok(0x01_18_3c)
    );
    assert_eq!(color_argument_value(&[VmValue::Integer(-1)]), Ok(0xff_ffff));
    assert!(color_argument_value(&[VmValue::Integer(1), VmValue::Integer(2)]).is_err());
    assert!(
        color_argument_value(&[
            VmValue::Integer(256),
            VmValue::Integer(0),
            VmValue::Integer(0),
        ])
        .is_err()
    );
}
