package UpdateableLogLine;

use strict;

# a log line in the updateable log

sub new {
  my ($class,
      $name,		# a unique name
      $message,		# the current log message
      $value,           # a sortable value
      $value2,		# a second sortable value, used when the first values are equal
     )
      = @_;

  my $self =
    {
     name => $name,
     message => $message,
     value => $value,
     value2 => $value2,
    };

  return bless $self, $class;
}

sub copyFrom($$) {
  my ($self, $other) = @_;

  $self->{name} = $other->{name};
  $self->{message} = $other->{message};
  $self->{value} = $other->{value};
  $self->{value2} = $other->{value2};
}

1;
