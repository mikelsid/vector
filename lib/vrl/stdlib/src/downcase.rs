use vrl::{function::VmArgumentList, prelude::*};

#[derive(Clone, Copy, Debug)]
pub struct Downcase;

impl Function for Downcase {
    fn identifier(&self) -> &'static str {
        "downcase"
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[Parameter {
            keyword: "value",
            kind: kind::BYTES,
            required: true,
        }]
    }

    fn compile(
        &self,
        _state: &state::Compiler,
        _ctx: &FunctionCompileContext,
        mut arguments: ArgumentList,
    ) -> Compiled {
        let value = arguments.required("value");

        Ok(Box::new(DowncaseFn { value }))
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            title: "downcase",
            source: r#"downcase("FOO 2 BAR")"#,
            result: Ok("foo 2 bar"),
        }]
    }

    fn call(&self, mut args: VmArgumentList) -> Value {
        let value = args.required("value");
        value.try_bytes_utf8_lossy().unwrap().to_lowercase().into()
    }
}

#[derive(Debug, Clone)]
struct DowncaseFn {
    value: Box<dyn Expression>,
}

#[no_mangle]
pub extern "C" fn vrl_fn_downcase(value: &mut Resolved, resolved: &mut Resolved) {
    let value = {
        let mut moved = Ok(Value::Null);
        std::mem::swap(value, &mut moved);
        moved
    };

    *resolved = (|| {
        let bytes = value?.try_bytes()?;
        Ok(String::from_utf8_lossy(&bytes).to_lowercase().into())
    })();
}

#[used]
static FN: extern "C" fn(&mut Resolved, &mut Resolved) = vrl_fn_downcase;

impl Expression for DowncaseFn {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        let bytes = self.value.resolve(ctx)?.try_bytes()?;

        Ok(String::from_utf8_lossy(&bytes).to_lowercase().into())
    }

    fn type_def(&self, _: &state::Compiler) -> TypeDef {
        TypeDef::new().bytes().infallible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    test_function![
        downcase => Downcase;

        simple {
            args: func_args![value: "FOO 2 bar"],
            want: Ok(value!("foo 2 bar")),
            tdef: TypeDef::new().bytes(),
        }
    ];
}
