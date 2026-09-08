# ASB terminal settings wizard

`asb-tui` is an independent frontend. Its state model accepts only choices
advertised by a negotiated runner catalog, keeps credential values
unrepresentable, and separates validation and plan creation from launch.

This bounded slice supplies the settings model and plain startup shell.
Interactive rendering, transport wiring, and run control remain follow-up work;
this crate does not claim those surfaces.
