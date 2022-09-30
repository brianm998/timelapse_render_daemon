package UpdateableLogLine;

use strict;

# a log line in the updateable log

sub new {
  my ($class,
      $name,		# a unique name
      $message,		# the current log message
     )
      = @_;

  my $self =
    {
     'name', $name,
    };

  return bless $self, $class;
}

1;
