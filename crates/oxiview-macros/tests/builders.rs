//! Compiles expanded Builders and props against a small method-level fixture.

extern crate self as oxiview_core;
use oxiview_macros::{Props, view};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
struct View {
    style: u32,
    children: Vec<View>,
    factory: Option<Box<dyn FnOnce() -> Result<View, ()>>>,
    on_pointer: Option<Box<dyn FnMut(u32)>>,
    binding: Option<Box<dyn FnMut() -> u32>>,
}
fn node() -> View {
    View::default()
}
fn component(factory: impl FnOnce() -> Result<View, ()> + 'static) -> View {
    View {
        factory: Some(Box::new(factory)),
        ..View::default()
    }
}
impl View {
    fn style(mut self, value: u32) -> Self {
        self.style = value;
        self
    }
    fn child(mut self, child: View) -> Self {
        self.children.push(child);
        self
    }
    fn on_pointer(mut self, handler: impl FnMut(u32) + 'static) -> Self {
        self.on_pointer = Some(Box::new(handler));
        self
    }
    fn bind_style(mut self, binding: impl FnMut() -> u32 + 'static) -> Self {
        self.binding = Some(Box::new(binding));
        self
    }
    fn mount(mut self) -> Result<Self, ()> {
        if let Some(factory) = self.factory.take() {
            let mut view = factory()?;
            view.children.extend(self.children);
            return view.mount();
        }
        self.children = self
            .children
            .into_iter()
            .map(View::mount)
            .collect::<Result<_, _>>()?;
        if let Some(binding) = self.binding.as_mut() {
            self.style = binding();
        }
        Ok(self)
    }
}

#[derive(Props)]
struct PanelProps {
    value: u32,
    calls: Rc<Cell<u32>>,
    #[prop(default = 4)]
    extra: u32,
    #[prop(default)]
    optional: Option<String>,
}
fn panel(props: PanelProps) -> View {
    props.calls.set(props.calls.get() + 1);
    node().style(props.value + props.extra + u32::from(props.optional.is_some()))
}

#[test]
fn nested_nodes_components_events_and_expression_children_lower_to_builders()
-> Result<(), Box<dyn std::error::Error>> {
    let calls = Rc::new(Cell::new(0));
    let count = Rc::clone(&calls);
    let event_value = Rc::new(Cell::new(0));
    let target = Rc::clone(&event_value);
    let dynamic = Rc::new(Cell::new(7));
    let binding = Rc::clone(&dynamic);
    let props = PanelProps::builder().value(10).calls(count).build()?;
    let view = view! {
        <Node style={{let base=2;base+3}} on_pointer={move |value|target.set(value)}>
            <panel props={props}><Node style={2}/></panel>
            <Node bind_style={move ||binding.get()}/>
            {node().style(9)}
        </Node>
    };
    assert_eq!(calls.get(), 0);
    let mut mounted = view.mount().map_err(|_| "fixture mount failed")?;
    assert_eq!(calls.get(), 1);
    assert_eq!(mounted.style, 5);
    assert_eq!(mounted.children.len(), 3);
    let first = mounted.children.first().ok_or("missing component child")?;
    assert_eq!(first.style, 14);
    assert_eq!(first.children.len(), 1);
    if let Some(handler) = mounted.on_pointer.as_mut() {
        handler(42);
    }
    assert_eq!(event_value.get(), 42);
    dynamic.set(8);
    let second = mounted.children.get_mut(1).ok_or("missing bound child")?;
    if let Some(binding) = second.binding.as_mut() {
        assert_eq!(binding(), 8);
    }
    assert_eq!(calls.get(), 1);
    Ok(())
}

#[derive(Clone, Props)]
struct GenericProps<T>
where
    T: Clone,
{
    value: T,
    #[prop(default)]
    extras: Vec<T>,
}

#[test]
fn props_builder_supports_generics_defaults_and_checked_missing_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let missing = PanelProps::builder().value(3).build();
    assert!(matches!(
        missing,
        Err(PanelPropsBuildError { field: "calls" })
    ));
    let built = GenericProps::builder()
        .value(String::from("hello"))
        .build()?;
    assert_eq!(built.value, "hello");
    assert!(built.extras.is_empty());
    let overridden = GenericProps::builder()
        .value(1)
        .extras(vec![2, 3])
        .build()?;
    assert_eq!(overridden.extras, vec![2, 3]);
    Ok(())
}

mod renamed {
    pub(super) fn node() -> super::View {
        super::node()
    }
}
#[test]
fn an_explicit_core_path_supports_dependency_renames() {
    let view = view! {crate=crate::renamed;<Node style={12}/>};
    assert_eq!(view.style, 12);
}

#[derive(Props)]
struct DirectProps {
    value: u32,
}
#[test]
fn deriving_props_does_not_require_using_its_optional_builder() {
    let props = DirectProps { value: 5 };
    assert_eq!(props.value, 5);
}

mod authored {
    oxiview_macros::oxi_module!("tests/sources/named.oxi");
}

#[test]
fn four_source_forms_bind_and_dispatch_identically() -> Result<(), Box<dyn std::error::Error>> {
    type Button = fn(Rc<Cell<u32>>, Rc<Cell<u32>>) -> View;
    let forms: [Button; 4] = [
        |value, target| {
            node()
                .bind_style(move || value.get())
                .on_pointer(move |n| target.set(n))
                .child(node().style(3))
        },
        |value, target| {
            view! {
                <Node bind_style={move || value.get()} on_pointer={move |n| target.set(n)}>
                    <Node style={3}/>
                </Node>
            }
        },
        authored::button,
        |value, target| oxiview_macros::oxi_view!("tests/sources/button.oxi"),
    ];
    for form in forms {
        let value = Rc::new(Cell::new(5));
        let target = Rc::new(Cell::new(0));
        let mut mounted = form(Rc::clone(&value), Rc::clone(&target))
            .mount()
            .map_err(|_| "mount failed")?;
        assert_eq!(mounted.style, 5);
        assert_eq!(mounted.children.len(), 1);
        assert_eq!(mounted.children.first().ok_or("missing child")?.style, 3);
        value.set(12);
        assert_eq!(mounted.binding.as_mut().ok_or("missing binding")?(), 12);
        mounted.on_pointer.as_mut().ok_or("missing handler")?(42);
        assert_eq!(target.get(), 42);
    }
    Ok(())
}
