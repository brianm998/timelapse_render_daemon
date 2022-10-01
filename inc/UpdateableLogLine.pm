package UpdateableLogLine;

use strict;

# a log line in the updateable log

sub new {
  my ($class,
      $name,		# a unique name
      $message,		# the current log message
      $value,           # a sortable value
     )
      = @_;

  my $self =
    {
     'name', $name,
     'message', $message,
     'value', $value,
    };

  return bless $self, $class;
}

1;
