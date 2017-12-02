`night-device-report` is a cronjob that performs various health checks on Debian-like systems, then reports the results to `night`.

`night` is my private status monitor system.

# Requirements

* Python 3
* `cron-apt`
* `needrestart`
* [python-xdg-basedir](https://github.com/fenhl/python-xdg-basedir)
* [requests](http://docs.python-requests.org/en/stable/)
