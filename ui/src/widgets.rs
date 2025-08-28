use std::fmt::Display;
use std::str::FromStr;

use xilem::WidgetView;
use xilem::style::{Padding, Style};
use xilem::view::{TextInput, sized_box, text_input};

pub fn margin<State: 'static, Action: 'static, V: 'static>(
    inner: V,
    margin: Padding,
) -> impl WidgetView<State, Action> + use<State, Action, V>
where
    V: WidgetView<State, Action>,
{
    sized_box(inner).padding(margin)
}

#[derive(Clone)]
pub struct NumberInputState<N> {
    pub raw: String,
    pub last_valid: N,
}
impl<N: Display> NumberInputState<N> {
    pub fn new(value: N) -> Self {
        NumberInputState {
            raw: value.to_string(),
            last_valid: value,
        }
    }
}

pub fn number_input<N, F, State, Action>(
    contents: NumberInputState<N>,
    on_changed: F,
) -> TextInput<State, Action>
where
    F: Fn(&mut State, NumberInputState<N>) -> Action + Send + Sync + 'static,
    N: Send + Sync + 'static,
    N: FromStr + Clone,
{
    text_input(contents.raw, move |state, text| {
        // TODO: only works for integer types
        let text: String = text.chars().filter(|c| c.is_numeric()).collect();
        on_changed(
            state,
            NumberInputState {
                last_valid: text.parse::<N>().unwrap_or(contents.last_valid.clone()),
                raw: text,
            },
        )
    })
}
