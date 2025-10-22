pub mod number_input;
pub mod progress_bar_integer;

use xilem::WidgetView;
use xilem::style::{Padding, Style};
use xilem::view::sized_box;

pub fn margin<State: 'static, Action: 'static, V>(
    inner: V,
    margin: Padding,
) -> impl WidgetView<State, Action> + use<State, Action, V>
where
    V: WidgetView<State, Action>,
{
    sized_box(inner).padding(margin)
}
