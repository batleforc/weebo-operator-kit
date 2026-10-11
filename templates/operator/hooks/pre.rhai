// Derived placeholders, for file names (a Liquid filter there would put a
// `|` in the template's paths).
variable::set("product_lower", variable::get("product").to_lower());
