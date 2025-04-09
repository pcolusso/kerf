# kerf

Cut through your logging crimes!

When you log naivley to CloudWatch, each line becomes it's own entry. You may be used to simply searching a log file, and just scrolling, but CloudWatch instead offers filtering, so if you find a line in a stack trace, you lack the other lines to actually do something about it.

This is a simple tool to punch in a filter, and view the logs around it.

## TODO:

- [x] Create CW iface
- [x] Search for CW logs by search term
- [x] Select log, find ctx around it, show in popup.
- [ ] Pagination
- [ ] Custom time ranges
