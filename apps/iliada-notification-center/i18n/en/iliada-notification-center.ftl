notifications = Notifications
no-notifications = No notifications
clear = Clear
clear-all = Clear All
dismiss = Dismiss
show-less = Show less
show-all = Show all { $count }
more = { $count ->
    [one] { $count } more
   *[other] { $count } more
}
do-not-disturb = Do Not Disturb
dnd-off = Off
dnd-on = On until you turn it off
dnd-on-until = On until { $time }
dnd-on-schedule = On until { $time } (schedule)
ago-now = Just now
ago-minutes = { $count ->
    [one] { $count } minute ago
   *[other] { $count } minutes ago
}
ago-hours = { $count ->
    [one] { $count } hour ago
   *[other] { $count } hours ago
}
ago-days = { $count ->
    [one] { $count } day ago
   *[other] { $count } days ago
}
