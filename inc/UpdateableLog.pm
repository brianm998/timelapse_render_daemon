package UpdateableLog;

use UpdateableLogLine;
use Term::ReadKey;

use strict;

use Term::ANSIColor qw(:constants);

# a log system that allows updates to log lines after logging

sub new {
  my ($class) = @_;

  my $self =
    {
     'list', [],		# a list of log lines by position on screen
    };

  return bless $self, $class;
}

sub sort($) {
  my ($self) = @_;
  $self->clear();
  my @sorted_logs = sort {
    if($a->{value} == $b->{value}) {
      return $a->{value2} <=> $b->{value2}
    } else {
      return $a->{value} <=> $b->{value}
    }
  } @{$self->{list}};
  $self->{list} = \@sorted_logs;
  $self->redraw();
}

sub redraw($) {
  my ($self) = @_;

  my $index = scalar(@{$self->{list}});
  foreach my $line (@{$self->{list}}) {
    print "\033[$index"."A";	# move cursor up $index lines
    print $line->{message};
    print "\n";
    print "\033[$index"."B";  # move cursor down $index lines
    $index--;
  }
}

sub clear($) {
  my ($self) = @_;

  my ($screen_char_width) = GetTerminalSize();

  my $index = scalar(@{$self->{list}});
  foreach my $line (@{$self->{list}}) {  
    print "\033[$index"."A";	# move cursor up $index lines
    for(my $i = 0 ; $i < $screen_char_width ; $i++) {
      print " ";
    }
    print "\n";
    print "\033[$index"."B";  # move cursor down $index lines
    $index--;
  }
}

sub timeLog($$$) {
  my ($self, $name, $message, $value) = @_;

  my $d = `date "+%r"`;
  chomp $d;

  $self->log($name, YELLOW.$d.RESET." - $message", $value, time);
}

sub log($$$) {
  my ($self, $name, $message, $value1, $value2) = @_;

  $message =~ s/\n//g;

  my $found = 0;
  my $new_logline = UpdateableLogLine->new($name, $message, $value1, $value2);

  my $index = scalar(@{$self->{list}});
  # first look at is it in the list
  foreach my $line (@{$self->{list}}) {
    if($line->{name} eq $name) {
      $found = 1;
      $line->copyFrom($new_logline);
    }
    $index--;
  }

  if(!$found) {
    # add to end of list by printing a blank line here
    push @{$self->{list}}, $new_logline;
    print "\n";
  }

  # lastly sort and redisplay
  $self->sort();
}

1;
